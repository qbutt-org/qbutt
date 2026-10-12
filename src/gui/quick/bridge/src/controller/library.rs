use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use cxx_qt::{CxxQtThread, CxxQtType, Threading};
use cxx_qt_lib::QString;
use qbutt_content::translation::context::{ContextAdditions, EvidenceSegment, TranslationContext};
use qbutt_content::translation::manga::PageRegion;
use qbutt_content::translation::text::{cache_key, restore_cached};
use qbutt_content::{
    DownloadJob, DownloadStorage, Library, LibraryEntry, LibraryError, LibraryResult, LibraryView,
    LocalBook, PublicationRef, ReaderProgress, ReaderSettings, SourceSettings, TextChapter,
    TranslationCacheKey, VariantPreferences, Work, WorkGroup,
};
use tokio::runtime::Runtime;
use tokio::sync::mpsc::{self, Sender};
use tokio::sync::oneshot;

use super::ffi;
use super::ffi::ContentController;
use super::playback::PlaybackSave;
use super::sources::SourceWrite;
use crate::models;

pub enum LibraryAction {
    LocalBookImport {
        book: Arc<LocalBook>,
        generation: u64,
    },
    LocalBookPath {
        reference: PublicationRef,
        generation: u64,
    },
    PlaybackHistory {
        token: u64,
        save: PlaybackSave,
    },
    SourceSettings,
    SourceWrite {
        write: SourceWrite,
        generation: u64,
    },
    Favorite {
        group: WorkGroup,
        favorite: bool,
    },
    Preferences {
        group: WorkGroup,
        preferences: VariantPreferences,
    },
    ReaderProgress {
        save: ReaderSave,
        generation: u64,
    },
    ReaderSettings {
        settings: ReaderSettings,
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
    pub group: WorkGroup,
    pub preferences: VariantPreferences,
    pub progress: ReaderProgress,
}

pub enum LibraryUpdate {
    LocalBookImport {
        book: Arc<LocalBook>,
        result: LibraryResult<()>,
        generation: u64,
    },
    LocalBookPath {
        reference: PublicationRef,
        result: LibraryResult<Option<PathBuf>>,
        generation: u64,
    },
    Loaded {
        result: LibraryResult<(Vec<LibraryEntry>, Vec<Work>, ReaderSettings)>,
        sources: Option<LibraryResult<Vec<SourceSettings>>>,
        error_generation: u64,
        reader_generation: Option<u64>,
        settings_retry_generation: Option<u64>,
    },
    Changed {
        result: LibraryResult<Option<Box<LibraryEntry>>>,
        history: Option<LibraryResult<Vec<Work>>>,
        error_generation: u64,
        reader_generation: Option<u64>,
        playback_token: Option<u64>,
    },
    ReaderResume {
        result: LibraryResult<Option<ReaderProgress>>,
        generation: u64,
    },
    ReaderSettings {
        result: LibraryResult<()>,
        generation: u64,
    },
    History {
        result: LibraryResult<Vec<Work>>,
        error_generation: u64,
    },
    SourceSettings {
        result: LibraryResult<Vec<SourceSettings>>,
    },
    SourceWrite {
        write: SourceWrite,
        generation: u64,
        result: LibraryResult<SourceSettings>,
    },
}

struct LibraryRequest {
    action: LibraryAction,
    error_generation: u64,
}

enum LibraryCommand {
    Request(Box<LibraryRequest>),
    DownloadJobs(oneshot::Sender<LibraryResult<Vec<DownloadJob>>>),
    SaveDownload(DownloadJob, oneshot::Sender<LibraryResult<()>>),
    TranslationLookup {
        original: Arc<TextChapter>,
        language: String,
        model: String,
        effort: String,
        reply: oneshot::Sender<LibraryResult<TranslationLookup>>,
    },
    TranslationSave {
        key: TranslationCacheKey,
        chapter: TextChapter,
        additions: ContextAdditions,
        evidence: Vec<EvidenceSegment>,
        reply: oneshot::Sender<LibraryResult<TranslationContext>>,
    },
    MangaLookup {
        key: TranslationCacheKey,
        reply: oneshot::Sender<LibraryResult<MangaTranslationLookup>>,
    },
    MangaSave {
        key: TranslationCacheKey,
        regions: Vec<PageRegion>,
        reply: oneshot::Sender<LibraryResult<TranslationContext>>,
    },
    Retry {
        error_generation: u64,
        reader_generation: Option<u64>,
        settings_retry_generation: Option<u64>,
    },
}

#[derive(Clone)]
pub struct LibraryWorker {
    sender: Sender<LibraryCommand>,
}

pub(super) struct TranslationLookup {
    pub key: TranslationCacheKey,
    pub context: TranslationContext,
    pub cached: Option<TextChapter>,
}

pub(super) struct MangaTranslationLookup {
    pub key: TranslationCacheKey,
    pub context: TranslationContext,
    pub cached: Option<Vec<PageRegion>>,
}

#[derive(Clone)]
pub(super) struct DownloadStore {
    sender: Sender<LibraryCommand>,
}

impl DownloadStorage for DownloadStore {
    async fn load(&self) -> LibraryResult<Vec<DownloadJob>> {
        let (reply, received) = oneshot::channel();
        self.sender
            .send(LibraryCommand::DownloadJobs(reply))
            .await
            .map_err(|_| LibraryError::InvalidEntry("download storage closed"))?;
        received
            .await
            .map_err(|_| LibraryError::InvalidEntry("download storage unavailable"))?
    }

    async fn save(&self, job: DownloadJob) -> LibraryResult<()> {
        let (reply, received) = oneshot::channel();
        self.sender
            .send(LibraryCommand::SaveDownload(job, reply))
            .await
            .map_err(|_| LibraryError::InvalidEntry("download storage closed"))?;
        received
            .await
            .map_err(|_| LibraryError::InvalidEntry("download storage unavailable"))?
    }
}

impl LibraryWorker {
    pub(super) async fn translation_lookup(
        &self,
        original: Arc<TextChapter>,
        language: String,
        model: String,
        effort: String,
    ) -> LibraryResult<TranslationLookup> {
        let (reply, received) = oneshot::channel();
        self.sender
            .send(LibraryCommand::TranslationLookup {
                original,
                language,
                model,
                effort,
                reply,
            })
            .await
            .map_err(|_| LibraryError::InvalidEntry("translation library is unavailable"))?;
        received
            .await
            .map_err(|_| LibraryError::InvalidEntry("translation library is unavailable"))?
    }

    pub(super) async fn save_translation(
        &self,
        key: TranslationCacheKey,
        chapter: TextChapter,
        additions: ContextAdditions,
        evidence: Vec<EvidenceSegment>,
    ) -> LibraryResult<TranslationContext> {
        let (reply, received) = oneshot::channel();
        self.sender
            .send(LibraryCommand::TranslationSave {
                key,
                chapter,
                additions,
                evidence,
                reply,
            })
            .await
            .map_err(|_| LibraryError::InvalidEntry("translation library is unavailable"))?;
        received
            .await
            .map_err(|_| LibraryError::InvalidEntry("translation library is unavailable"))?
    }

    pub(super) async fn manga_lookup(
        &self,
        key: TranslationCacheKey,
    ) -> LibraryResult<MangaTranslationLookup> {
        let (reply, received) = oneshot::channel();
        self.sender
            .send(LibraryCommand::MangaLookup { key, reply })
            .await
            .map_err(|_| LibraryError::InvalidEntry("translation library is unavailable"))?;
        received
            .await
            .map_err(|_| LibraryError::InvalidEntry("translation library is unavailable"))?
    }

    pub(super) async fn save_manga_translation(
        &self,
        key: TranslationCacheKey,
        regions: Vec<PageRegion>,
    ) -> LibraryResult<TranslationContext> {
        let (reply, received) = oneshot::channel();
        self.sender
            .send(LibraryCommand::MangaSave {
                key,
                regions,
                reply,
            })
            .await
            .map_err(|_| LibraryError::InvalidEntry("translation library is unavailable"))?;
        received
            .await
            .map_err(|_| LibraryError::InvalidEntry("translation library is unavailable"))?
    }

    pub(super) fn download_store(&self) -> DownloadStore {
        DownloadStore {
            sender: self.sender.clone(),
        }
    }

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
                controller.as_mut().reader_settings_changed();
            });
            let mut error_generation = 0;
            let mut reader_generation = None;
            let mut settings_retry_generation = None;
            loop {
                let (mut library, result, sources) = match Library::open(&profile) {
                    Ok(library) => {
                        let result = library.entries(LibraryView::All).and_then(|entries| {
                            Ok((entries, read_history(&library)?, library.reader_settings()?))
                        });
                        let sources = library.source_settings();
                        (Some(library), result, Some(sources))
                    }
                    Err(error) => (None, Err(error), None),
                };
                let _ = qt_thread.queue(move |controller| {
                    controller.complete_library(LibraryUpdate::Loaded {
                        result,
                        sources,
                        error_generation,
                        reader_generation,
                        settings_retry_generation,
                    });
                });

                // This job remains the sole connection owner after a failed
                // open. An explicit retry reopens the same profile in FIFO order.
                loop {
                    let Some(command) = receiver.blocking_recv() else {
                        return;
                    };
                    match command {
                        LibraryCommand::DownloadJobs(reply) => {
                            let result = library
                                .as_ref()
                                .ok_or(LibraryError::InvalidEntry("download storage unavailable"))
                                .and_then(Library::download_jobs);
                            let _ = reply.send(result);
                        }
                        LibraryCommand::SaveDownload(job, reply) => {
                            let result = library
                                .as_mut()
                                .ok_or(LibraryError::InvalidEntry("download storage unavailable"))
                                .and_then(|library| library.save_download_job(&job));
                            let _ = reply.send(result);
                        }
                        LibraryCommand::TranslationLookup {
                            original,
                            language,
                            model,
                            effort,
                            reply,
                        } => {
                            let result = library
                                .as_ref()
                                .ok_or(LibraryError::InvalidEntry(
                                    "translation library is unavailable",
                                ))
                                .and_then(|library| {
                                    let context = library.translation_context(
                                        &original.reference.variant.publication,
                                        &language,
                                    )?;
                                    let key = cache_key(
                                        &original,
                                        &language,
                                        &model,
                                        &effort,
                                        context.revision,
                                    )?;
                                    let cached = library
                                        .cached_translation::<TextChapter>(&key)?
                                        .map(|chapter| {
                                            restore_cached(&original, chapter).map_err(|_| {
                                                LibraryError::CorruptEntry(
                                                    "cached text translation is invalid",
                                                )
                                            })
                                        })
                                        .transpose()?;
                                    Ok(TranslationLookup {
                                        key,
                                        context,
                                        cached,
                                    })
                                });
                            let _ = reply.send(result);
                        }
                        LibraryCommand::TranslationSave {
                            key,
                            chapter,
                            additions,
                            evidence,
                            reply,
                        } => {
                            let result = library
                                .as_mut()
                                .ok_or(LibraryError::InvalidEntry(
                                    "translation library is unavailable",
                                ))
                                .and_then(|library| {
                                    library.save_translation(key, &chapter, additions, &evidence)
                                });
                            let _ = reply.send(result);
                        }
                        LibraryCommand::MangaLookup { mut key, reply } => {
                            let result = library
                                .as_ref()
                                .ok_or(LibraryError::InvalidEntry(
                                    "translation library is unavailable",
                                ))
                                .and_then(|library| {
                                    let context = library.translation_context(
                                        &key.chapter.variant.publication,
                                        &key.target_language,
                                    )?;
                                    key.context_revision = context.revision;
                                    let cached =
                                        library.cached_translation::<Vec<PageRegion>>(&key)?;
                                    Ok(MangaTranslationLookup {
                                        key,
                                        context,
                                        cached,
                                    })
                                });
                            let _ = reply.send(result);
                        }
                        LibraryCommand::MangaSave {
                            key,
                            regions,
                            reply,
                        } => {
                            let result = library
                                .as_mut()
                                .ok_or(LibraryError::InvalidEntry(
                                    "translation library is unavailable",
                                ))
                                .and_then(|library| {
                                    library.save_translation(
                                        key,
                                        &regions,
                                        ContextAdditions::default(),
                                        &[],
                                    )
                                });
                            let _ = reply.send(result);
                        }
                        LibraryCommand::Retry {
                            error_generation: retry_error_generation,
                            reader_generation: retry_reader_generation,
                            settings_retry_generation: retry_settings_generation,
                        } => {
                            error_generation = retry_error_generation;
                            reader_generation = retry_reader_generation;
                            settings_retry_generation = retry_settings_generation;
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
        settings_retry_generation: Option<u64>,
    ) -> Result<(), &'static str> {
        self.sender
            .try_send(LibraryCommand::Retry {
                error_generation,
                reader_generation,
                settings_retry_generation,
            })
            .map_err(|_| "The content library retry queue is full or closed")
    }
}

fn apply(library: &mut Library, request: LibraryRequest) -> LibraryUpdate {
    let (result, reader_generation, playback_token) = match request.action {
        LibraryAction::LocalBookImport { book, generation } => {
            return LibraryUpdate::LocalBookImport {
                result: library.save_local_book(&book),
                book,
                generation,
            };
        }
        LibraryAction::LocalBookPath {
            reference,
            generation,
        } => {
            return LibraryUpdate::LocalBookPath {
                result: library.local_book_path(&reference),
                reference,
                generation,
            };
        }
        LibraryAction::PlaybackHistory { token, save } => (
            library
                .record_opened(&save.group, save.variant, save.preferences)
                .map(Some),
            None,
            Some(token),
        ),
        LibraryAction::SourceSettings => {
            return LibraryUpdate::SourceSettings {
                result: library.source_settings(),
            };
        }
        LibraryAction::SourceWrite { write, generation } => {
            let result = match &write {
                SourceWrite::Enabled { source, enabled } => {
                    library.set_source_enabled(*source, *enabled)
                }
                SourceWrite::Failure(failure) => library.record_source_failure(failure.clone()),
                SourceWrite::Clear {
                    operation,
                    observed_revision,
                } => library.clear_source_failure(operation, *observed_revision),
            };
            return LibraryUpdate::SourceWrite {
                write,
                generation,
                result,
            };
        }
        LibraryAction::Favorite { group, favorite } => {
            (library.set_favorite(&group, favorite).map(Some), None, None)
        }
        LibraryAction::Preferences { group, preferences } => (
            library.save_preferences(&group, preferences).map(Some),
            None,
            None,
        ),
        LibraryAction::ReaderProgress { save, generation } => (
            library
                .save_reader_progress(&save.group, save.preferences, save.progress)
                .map(Some),
            Some(generation),
            None,
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
        LibraryAction::ReaderSettings {
            settings,
            generation,
        } => {
            return LibraryUpdate::ReaderSettings {
                result: library.save_reader_settings(settings),
                generation,
            };
        }
        LibraryAction::History => {
            return LibraryUpdate::History {
                result: read_history(library),
                error_generation: request.error_generation,
            };
        }
        LibraryAction::RemoveHistory { reference } => {
            (library.remove_history(&reference), None, None)
        }
    };
    let history = result.as_ref().ok().map(|_| read_history(library));
    LibraryUpdate::Changed {
        result: result.map(|entry| entry.map(Box::new)),
        history,
        error_generation: request.error_generation,
        reader_generation,
        playback_token,
    }
}

fn read_history(library: &Library) -> LibraryResult<Vec<Work>> {
    library.entries(LibraryView::History).map(|entries| {
        entries
            .into_iter()
            .filter_map(|entry| entry.group.anchor_work().cloned())
            .collect()
    })
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

    pub(super) fn library_error_text(&self) -> QString {
        let error = match self.current_library_error_kind() {
            ffi::LibraryErrorKind::Opening => ffi::ContentError::LibraryOpen,
            ffi::LibraryErrorKind::Writing => ffi::ContentError::LibraryWrite,
            ffi::LibraryErrorKind::HistoryRead => ffi::ContentError::HistoryRead,
            ffi::LibraryErrorKind::PlaybackWriting => ffi::ContentError::PlaybackHistoryWrite,
            _ => return QString::default(),
        };
        ffi::content_error(error)
    }

    pub(super) fn current_library_error_kind(&self) -> ffi::LibraryErrorKind {
        if self.playback.failed() {
            ffi::LibraryErrorKind::PlaybackWriting
        } else {
            self.library_error_kind
        }
    }

    pub fn init_library(mut self: Pin<&mut Self>, profile_dir: QString) {
        if self.library_state != ffi::LibraryState::Uninitialized
            || self.library_worker.is_some()
            || self.source_settings_state() == ffi::SettingsState::Loading
        {
            return;
        }
        let profile = PathBuf::from(profile_dir.to_string());
        self.as_mut().source_settings_loading();
        if !profile.is_absolute() || self.services.is_none() {
            self.as_mut().rust_mut().library_state = ffi::LibraryState::Failed;
            self.as_mut().library_state_changed();
            self.as_mut().reader_settings_library_failed();
            self.as_mut().load_source_settings(None);
            self.as_mut()
                .set_library_error(ffi::LibraryErrorKind::Opening);
            return;
        }
        if let Ok(executable) = std::env::current_exe()
            && let Some(app_dir) = executable.parent()
        {
            self.services
                .as_ref()
                .unwrap()
                .catalog
                .initialize_webview(profile.clone(), app_dir.to_path_buf());
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
        let settings_retry_generation = self.reader_settings_retry_generation();
        match worker.retry(
            self.library_error_generation,
            reader_generation,
            settings_retry_generation,
        ) {
            Ok(()) => {
                // The blocking owner has already entered and remains alive;
                // FIFO requests can be accepted behind this reopen command.
                self.as_mut().rust_mut().library_state = ffi::LibraryState::Loading;
                self.as_mut().source_settings_loading();
                if let Some(generation) = reader_generation {
                    self.as_mut().prepare_reader_library_retry(generation);
                }
                self.as_mut().library_state_changed();
                self.as_mut().reader_settings_changed();
                true
            }
            Err(error) => {
                ffi::log_content_error(&QString::from("library retry"), &QString::from(error));
                self.as_mut()
                    .set_library_error(ffi::LibraryErrorKind::Opening);
                false
            }
        }
    }

    pub(super) fn set_library_error(mut self: Pin<&mut Self>, kind: ffi::LibraryErrorKind) {
        if kind != ffi::LibraryErrorKind::None {
            self.as_mut().rust_mut().library_error_generation =
                self.library_error_generation.wrapping_add(1);
        }
        if self.library_error_kind != kind {
            self.as_mut().rust_mut().library_error_kind = kind;
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
        let mut settings_retry_generation = None;
        let mut initial_opened = false;
        let playback_write_failed = matches!(
            &update,
            LibraryUpdate::Changed {
                result: Err(_),
                playback_token: Some(_),
                ..
            }
        );
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
            LibraryUpdate::LocalBookImport {
                book,
                result,
                generation,
            } => {
                self.complete_local_book_import(book, result, generation);
                return;
            }
            LibraryUpdate::LocalBookPath {
                reference,
                result,
                generation,
            } => {
                self.complete_local_book_path(reference, result, generation);
                return;
            }
            LibraryUpdate::SourceSettings { result } => {
                self.load_source_settings(Some(result));
                return;
            }
            LibraryUpdate::SourceWrite {
                write,
                generation,
                result,
            } => {
                self.complete_source_write(write, generation, result, true);
                return;
            }
            LibraryUpdate::ReaderResume { result, generation } => {
                self.complete_reader_resume(result, generation);
                return;
            }
            LibraryUpdate::ReaderSettings { result, generation } => {
                self.complete_reader_settings(result, generation);
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
                sources,
                error_generation,
                reader_generation,
                settings_retry_generation: retry_generation,
            } => {
                initial_opened = sources.is_some();
                self.as_mut().load_source_settings(sources);
                settings_retry_generation = retry_generation;
                let result = result.map(|(entries, history, settings)| {
                    self.as_mut().load_reader_settings(settings);
                    for entry in &entries {
                        self.as_mut()
                            .rust_mut()
                            .work_preferences
                            .entry(entry.group.anchor.clone())
                            .or_insert_with(|| entry.preferences.clone());
                        self.as_mut()
                            .rust_mut()
                            .known_groups
                            .insert(entry.group.anchor.clone(), entry.group.clone());
                    }
                    for entry in entries.into_iter().filter(|entry| entry.favorite) {
                        if let Some(work) = entry.group.anchor_work() {
                            self.as_mut()
                                .rust_mut()
                                .favorites
                                .pin_mut()
                                .update(work.clone());
                        }
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
                playback_token,
            } => {
                if let Some(token) = playback_token {
                    self.as_mut()
                        .complete_playback_history(token, result.is_ok());
                }
                if let Some(generation) = reader_generation {
                    self.as_mut()
                        .complete_reader_progress(generation, result.is_ok());
                }
                let result = result.and_then(|entry| {
                    if let Some(entry) = entry {
                        self.as_mut()
                            .rust_mut()
                            .known_groups
                            .insert(entry.group.anchor.clone(), entry.group.clone());
                        if entry.favorite {
                            if let Some(work) = entry.group.anchor_work() {
                                self.as_mut()
                                    .rust_mut()
                                    .favorites
                                    .pin_mut()
                                    .update(work.clone());
                            }
                        } else {
                            self.as_mut()
                                .rust_mut()
                                .favorites
                                .pin_mut()
                                .remove(&entry.group.anchor);
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
        self.as_mut().refresh_selected_favorite();
        match result {
            Ok(()) => {
                if self.library_error_generation == error_generation {
                    self.as_mut().set_library_error(ffi::LibraryErrorKind::None);
                }
                if initial {
                    self.as_mut().rust_mut().library_state = ffi::LibraryState::Ready;
                    let generation = self.inspection.generation;
                    if let Some(group) = self.selected_group.clone()
                        && self.as_mut().set_selection(Some(&group), generation)
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
                    self.as_mut().reader_settings_changed();
                    if let Some(generation) = reader_generation {
                        self.as_mut().resume_reader_after_library_retry(generation);
                    }
                    if let Some(generation) = settings_retry_generation {
                        self.as_mut()
                            .resume_reader_settings_after_library_retry(generation);
                    }
                    if self.playback.failed() {
                        self.as_mut().retry_playback_history();
                    }
                }
            }
            Err(error) => {
                let reader_failed = if initial {
                    let mut state = self.as_mut().rust_mut();
                    state.library_state = ffi::LibraryState::Failed;
                    if !initial_opened {
                        state.library_pending = 0;
                        self.as_mut().playback_library_failed();
                    }
                    self.as_mut().reader_settings_library_failed();
                    self.as_mut().reader_library_failed()
                } else {
                    false
                };
                ffi::log_content_error(
                    &QString::from("library"),
                    &QString::from(error.to_string()),
                );
                if !playback_write_failed && self.library_error_generation == error_generation {
                    self.as_mut().set_library_error(if initial {
                        ffi::LibraryErrorKind::Opening
                    } else if history_read {
                        ffi::LibraryErrorKind::HistoryRead
                    } else {
                        ffi::LibraryErrorKind::Writing
                    });
                }
                if reader_failed {
                    self.as_mut().reader_changed();
                }
                if initial {
                    self.as_mut().library_state_changed();
                }
            }
        }
    }

    pub(super) fn submit_library(mut self: Pin<&mut Self>, mut action: LibraryAction) -> bool {
        if !matches!(
            self.library_state,
            ffi::LibraryState::Loading | ffi::LibraryState::Ready
        ) {
            return false;
        }
        // History keeps the exact confirmed content snapshot. Filter choices
        // belong to the work and can change while that confirmation is pending.
        let saved_filters = match &mut action {
            LibraryAction::PlaybackHistory { save, .. } => {
                Some((&save.group.anchor, &mut save.preferences.filters))
            }
            LibraryAction::ReaderProgress { save, .. } => {
                Some((&save.group.anchor, &mut save.preferences.filters))
            }
            _ => None,
        };
        if let Some((reference, filters)) = saved_filters
            && let Some(preferences) = self.work_preferences.get(reference)
        {
            filters.clone_from(&preferences.filters);
        }
        let error_kind = if matches!(&action, LibraryAction::History) {
            ffi::LibraryErrorKind::HistoryRead
        } else {
            ffi::LibraryErrorKind::Writing
        };
        let settings = matches!(
            &action,
            LibraryAction::ReaderSettings { .. }
                | LibraryAction::SourceSettings
                | LibraryAction::SourceWrite { .. }
                | LibraryAction::PlaybackHistory { .. }
        );
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
                if !settings {
                    self.as_mut().set_library_error(error_kind);
                }
                false
            }
        }
    }
}
