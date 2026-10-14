use std::collections::{HashMap, HashSet, VecDeque};
use std::pin::Pin;
use std::sync::Arc;

use cxx::UniquePtr;
use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use qbutt_content::{
    LocalBook, MediaTools, PublicationRef, SharedWork, VariantPreferences, Work, WorkAssociation,
    WorkGroup,
};
use tokio::runtime::{Builder, Runtime};
use tokio::task::JoinHandle;

mod browser;
mod capture;
mod catalog;
mod downloads;
mod inspection;
mod library;
mod local_books;
mod playback;
mod qcatch_setup;
mod reader;
pub(crate) mod reader_pages;
mod reader_settings;
mod search;
mod selection;
mod sharing;
mod sources;
mod translation_settings;
mod variant_filters;

use crate::download_model;
use crate::models;
use crate::source_model;
use downloads::CapturedIntent;
use library::{DownloadStore, LibraryUpdate, LibraryWorker};
use playback::PlaybackState;
use reader::ReaderState;
use reader_pages::{RetiredReaderCache, clean_retired_reader_cache};
use reader_settings::ReaderSettingsState;
use sources::SourceState;
use translation_settings::TranslationOptionsState;

#[cxx_qt::bridge(namespace = "Qbutt")]
pub mod ffi {
    #[derive(PartialEq)]
    #[repr(u8)]
    enum NativeHostState {
        Missing,
        Registered,
        Unknown,
    }

    struct BrowserInstallation {
        id: QString,
        name: QString,
        is_default: bool,
        installed: bool,
        native_host: NativeHostState,
    }
    #[derive(PartialEq)]
    #[repr(u8)]
    enum ContentError {
        ServicesUnavailable,
        Download,
        LibraryOpen,
        LibraryWrite,
        HistoryRead,
        ReaderSettingsWrite,
        Playback,
        PlaybackHistoryWrite,
        PlaybackPending,
        ExternalPlayback,
        BrowserPlayback,
    }

    #[derive(PartialEq)]
    #[repr(u8)]
    enum ReaderError {
        Chapters,
        Chapter,
        Page,
        ChapterUnavailable,
        PageUnavailable,
        ProgressWrite,
    }

    #[qml_element]
    qnamespace!("Qbutt::Search");
    #[qml_element]
    qnamespace!("Qbutt::Library");
    #[qml_element]
    qnamespace!("Qbutt::Reader");
    #[qml_element]
    qnamespace!("Qbutt::Sources");
    #[qml_element]
    qnamespace!("Qbutt::Browser");

    #[qenum]
    #[namespace = "Qbutt::Browser"]
    enum ConnectionState {
        Checking,
        Connected,
        Disconnected,
        ChoiceRequired,
        NoBrowser,
        OwnerExists,
        Unavailable,
    }

    #[qenum]
    #[namespace = "Qbutt::Browser"]
    enum SetupState {
        Idle,
        Busy,
        Waiting,
        Failed,
    }

    #[qenum]
    #[namespace = "Qbutt::Browser"]
    enum SetupFailure {
        None,
        Integrity,
        Download,
        Install,
        Connect,
    }

    #[qenum]
    #[namespace = "Qbutt::Sources"]
    enum SettingsState {
        Loading,
        Ready,
        Saving,
        Failed,
    }

    #[qenum]
    #[namespace = "Qbutt::Reader"]
    enum ReaderFit {
        Height,
        Width,
    }

    #[qenum]
    #[namespace = "Qbutt::Reader"]
    enum ReaderMode {
        Paged,
        Continuous,
    }

    #[qenum]
    #[namespace = "Qbutt::Reader"]
    enum ReaderPageState {
        PageUnloaded,
        PageLoading,
        PageReady,
        PageFailed,
    }

    #[qenum]
    #[namespace = "Qbutt::Reader"]
    enum ReaderSettingsState {
        Uninitialized,
        Loading,
        Ready,
        Saving,
        Failed,
    }

    #[qenum]
    #[namespace = "Qbutt::Reader"]
    enum TranslationSettingsError {
        NoTranslationSettingsError,
        InvalidTranslationModel,
        TranslationKeyProtectionFailed,
        TranslationSettingsStorageFailed,
    }

    #[qenum]
    #[namespace = "Qbutt::Search"]
    enum SearchState {
        Idle,
        Searching,
        Succeeded,
        Cancelled,
        Failed,
    }

    #[qenum]
    #[namespace = "Qbutt::Search"]
    enum ListMode {
        Search,
        Favorites,
        History,
    }

    #[qenum]
    #[namespace = "Qbutt::Library"]
    enum LibraryState {
        Uninitialized,
        Loading,
        Ready,
        Failed,
    }

    #[qenum]
    #[namespace = "Qbutt::Library"]
    enum LibraryErrorKind {
        None,
        Opening,
        Writing,
        HistoryRead,
        PlaybackWriting,
    }

    #[qenum]
    #[namespace = "Qbutt::Search"]
    enum VariantSelectionState {
        None,
        Inspecting,
        Available,
        Unavailable,
        SourceDisabled,
        UnsupportedPlayback,
    }

    #[namespace = ""]
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qurl.h");
        type QUrl = cxx_qt_lib::QUrl;
        include!("cxx-qt-lib/qvector.h");
        type QVector_QUrl = cxx_qt_lib::QVector<cxx_qt_lib::QUrl>;
        include!("cxx-qt-lib/qsize.h");
        type QSize = cxx_qt_lib::QSize;
        include!("cxx-qt-lib/qbytearray.h");
        type QByteArray = cxx_qt_lib::QByteArray;
    }

    unsafe extern "C++" {
        include!("qbutt_quick/src/models.cxxqt.h");
        type WorkModel = crate::models::ffi::WorkModel;
        type VariantModel = crate::models::ffi::VariantModel;
        include!("qbutt_quick/src/source_model.cxxqt.h");
        type SourceModel = crate::source_model::ffi::SourceModel;
        include!("qbutt_quick/src/download_model.cxxqt.h");
        type DownloadModel = crate::download_model::ffi::DownloadModel;
        include!("qbutt_quick/src/reader_models.cxxqt.h");
        type ChapterModel = crate::reader_models::ffi::ChapterModel;
        include!("qbutt_quick/src/text_model.cxxqt.h");
        type TextBlockModel = crate::text_model::ffi::TextBlockModel;
        include!("qbutt_quick/src/page_model.cxxqt.h");
        type PageModel = crate::page_model::ffi::PageModel;
        include!("qbutt_quick/native.h");
        #[rust_name = "content_error"]
        fn contentError(error: ContentError) -> QString;
        #[rust_name = "log_content_error"]
        fn logContentError(operation: &QString, detail: &QString);
        #[rust_name = "content_language"]
        fn contentLanguage() -> QString;
        #[rust_name = "reader_error"]
        fn readerError(error: ReaderError) -> QString;
        #[rust_name = "chapter_title"]
        fn chapterTitle(number: &QString, volume: &QString, title: &QString) -> QString;
        #[rust_name = "validated_image_size"]
        fn validatedImageSize(path: &QString) -> QSize;
        #[rust_name = "manga_translation_size_supported"]
        fn mangaTranslationSizeSupported(width: i32, height: i32) -> bool;
        #[rust_name = "retire_reader_cache"]
        fn retireReaderCache(cache: Box<RetiredReaderCache>);
        #[rust_name = "retire_content_services"]
        fn retireContentServices(services: Box<Services>);
        #[rust_name = "browser_installations"]
        fn browserInstallations() -> Vec<BrowserInstallation>;
        #[rust_name = "qcatch_executable"]
        fn qcatchExecutable() -> QString;
        #[rust_name = "open_browser_url"]
        fn openBrowserUrl(url: &QString, choice: &QString) -> bool;
        #[rust_name = "browser_discovery_path"]
        fn browserDiscoveryPath() -> QString;
        #[rust_name = "copy_share_url"]
        fn copyShareUrl(url: &QString);
    }

    extern "Rust" {
        type Services;
        type RetiredReaderCache;
        #[cxx_name = "cleanRetiredReaderCache"]
        fn clean_retired_reader_cache(cache: &RetiredReaderCache);
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(*mut WorkModel, works, READ = works_model, CONSTANT)]
        #[qproperty(*mut WorkModel, favorites, READ = favorites_model, CONSTANT)]
        #[qproperty(*mut WorkModel, history, READ = history_model, CONSTANT)]
        #[qproperty(QString, local_book_error, cxx_name = "localBookError", READ = local_book_error, NOTIFY = local_book_error_changed)]
        #[qproperty(*mut VariantModel, variants, READ = variants_model, CONSTANT)]
        #[qproperty(*mut DownloadModel, downloads, READ = downloads_model, CONSTANT)]
        #[qproperty(QString, downloads_state, cxx_name = "downloadsState", READ, NOTIFY)]
        #[qproperty(
            QString,
            variant_episode_filter,
            cxx_name = "variantEpisodeFilter",
            READ = variant_episode_filter,
            NOTIFY = variant_filters_changed
        )]
        #[qproperty(
            QString,
            variant_translation_filter,
            cxx_name = "variantTranslationFilter",
            READ = variant_translation_filter,
            NOTIFY = variant_filters_changed
        )]
        #[qproperty(
            QString,
            variant_source_filter,
            cxx_name = "variantSourceFilter",
            READ = variant_source_filter,
            NOTIFY = variant_filters_changed
        )]
        #[qproperty(
            QString,
            variant_player_filter,
            cxx_name = "variantPlayerFilter",
            READ = variant_player_filter,
            NOTIFY = variant_filters_changed
        )]
        #[qproperty(
            QString,
            variant_language_filter,
            cxx_name = "variantLanguageFilter",
            READ = variant_language_filter,
            NOTIFY = variant_filters_changed
        )]
        #[qproperty(
            QString,
            variant_transport_filter,
            cxx_name = "variantTransportFilter",
            READ = variant_transport_filter,
            NOTIFY = variant_filters_changed
        )]
        #[qproperty(
            QString,
            variant_query,
            cxx_name = "variantQuery",
            READ = variant_query,
            NOTIFY = variant_filters_changed
        )]
        #[qproperty(*mut SourceModel, sources, READ = sources_model, CONSTANT)]
        #[qproperty(QString, browser_choice, cxx_name = "browserChoice", READ = browser_choice, NOTIFY = browser_changed)]
        #[qproperty(ConnectionState, browser_connection_state, cxx_name = "browserConnectionState", READ = browser_connection_state, NOTIFY = browser_changed)]
        #[qproperty(QString, browser_connected_name, cxx_name = "browserConnectedName", READ = browser_connected_name, NOTIFY = browser_changed)]
        #[qproperty(SetupState, browser_setup_state, cxx_name = "browserSetupState", READ = browser_setup_state, NOTIFY = browser_changed)]
        #[qproperty(SetupFailure, browser_setup_failure, cxx_name = "browserSetupFailure", READ = browser_setup_failure, NOTIFY = browser_changed)]
        #[qproperty(QString, browser_extension_directory, cxx_name = "browserExtensionDirectory", READ = browser_extension_directory, NOTIFY = browser_changed)]
        #[qproperty(SettingsState, source_settings_state, cxx_name = "sourceSettingsState", READ = source_settings_state, NOTIFY = sources_changed)]
        #[qproperty(QString, source_settings_error_text, cxx_name = "sourceSettingsErrorText", READ = source_settings_error_text, NOTIFY = sources_changed)]
        #[qproperty(*mut PageModel, reader_pages, cxx_name = "readerPages", READ = reader_pages, CONSTANT)]
        #[qproperty(*mut TextBlockModel, reader_text_blocks, cxx_name = "readerTextBlocks", READ = reader_text_blocks, CONSTANT)]
        #[qproperty(bool, reader_is_text, cxx_name = "readerIsText", READ = reader_is_text, NOTIFY = reader_changed)]
        #[qproperty(u64, reader_text_generation, cxx_name = "readerTextGeneration", READ = reader_text_generation, NOTIFY = reader_changed)]
        #[qproperty(QString, reader_text_anchor, cxx_name = "readerTextAnchor", READ = reader_text_anchor, NOTIFY = reader_changed)]
        #[qproperty(f64, reader_text_offset, cxx_name = "readerTextOffset", READ = reader_text_offset, NOTIFY = reader_changed)]
        #[qproperty(bool, reader_translation_requested, cxx_name = "readerTranslationRequested", READ = reader_translation_requested, NOTIFY = reader_changed)]
        #[qproperty(bool, reader_translation_busy, cxx_name = "readerTranslationBusy", READ = reader_translation_busy, NOTIFY = reader_changed)]
        #[qproperty(bool, reader_translation_error, cxx_name = "readerTranslationError", READ = reader_translation_error, NOTIFY = reader_changed)]
        #[qproperty(bool, reader_translation_needs_codex, cxx_name = "readerTranslationNeedsCodex", READ = reader_translation_needs_codex, NOTIFY = reader_changed)]
        #[qproperty(bool, reader_translation_image_size_error, cxx_name = "readerTranslationImageSizeError", READ = reader_translation_image_size_error, NOTIFY = reader_changed)]
        #[qproperty(i32, reader_chapter_index, cxx_name = "readerChapterIndex", READ = reader_chapter_index, NOTIFY = reader_changed)]
        #[qproperty(i32, reader_chapter_count, cxx_name = "readerChapterCount", READ = reader_chapter_count, NOTIFY = reader_changed)]
        #[qproperty(ReaderMode, reader_mode, cxx_name = "readerMode", READ = reader_mode, NOTIFY = reader_changed)]
        #[qproperty(i32, reader_image_width, cxx_name = "readerImageWidth", READ = reader_image_width, NOTIFY = reader_changed)]
        #[qproperty(i32, reader_image_height, cxx_name = "readerImageHeight", READ = reader_image_height, NOTIFY = reader_changed)]
        #[qproperty(
            *mut ChapterModel,
            reader_chapters,
            cxx_name = "readerChapters",
            READ = reader_chapters,
            CONSTANT
        )]
        #[qproperty(
            bool,
            reader_open,
            cxx_name = "readerOpen",
            READ = reader_open,
            NOTIFY = reader_changed
        )]
        #[qproperty(
            bool,
            reader_has_chapters,
            cxx_name = "readerHasChapters",
            READ = reader_has_chapters,
            NOTIFY = reader_changed
        )]
        #[qproperty(
            bool,
            reader_busy,
            cxx_name = "readerBusy",
            READ = reader_busy,
            NOTIFY = reader_changed
        )]
        #[qproperty(
            QString,
            reader_error_text,
            cxx_name = "readerErrorText",
            READ = reader_error_text,
            NOTIFY = reader_changed
        )]
        #[qproperty(
            QString,
            reader_progress_error_text,
            cxx_name = "readerProgressErrorText",
            READ = reader_progress_error_text,
            NOTIFY = reader_changed
        )]
        #[qproperty(
            bool,
            reader_progress_pending,
            cxx_name = "readerProgressPending",
            READ = reader_progress_pending,
            NOTIFY = reader_changed
        )]
        #[qproperty(
            QString,
            reader_title,
            cxx_name = "readerTitle",
            READ = reader_title,
            NOTIFY = reader_changed
        )]
        #[qproperty(
            QString,
            reader_chapter_title,
            cxx_name = "readerChapterTitle",
            READ = reader_chapter_title,
            NOTIFY = reader_changed
        )]
        #[qproperty(
            QString,
            reader_chapter_id,
            cxx_name = "readerChapterId",
            READ = reader_chapter_id,
            NOTIFY = reader_changed
        )]
        #[qproperty(
            QUrl,
            reader_page_url,
            cxx_name = "readerPageUrl",
            READ = reader_page_url,
            NOTIFY = reader_changed
        )]
        #[qproperty(
            i32,
            reader_page_index,
            cxx_name = "readerPageIndex",
            READ = reader_page_index,
            NOTIFY = reader_changed
        )]
        #[qproperty(
            i32,
            reader_page_count,
            cxx_name = "readerPageCount",
            READ = reader_page_count,
            NOTIFY = reader_changed
        )]
        #[qproperty(
            f64,
            reader_scroll_offset,
            cxx_name = "readerScrollOffset",
            READ = reader_scroll_offset,
            NOTIFY = reader_changed
        )]
        #[qproperty(
            ReaderFit,
            reader_fit,
            cxx_name = "readerFit",
            READ = reader_fit,
            NOTIFY = reader_changed
        )]
        #[qproperty(
            QString,
            reader_background_color,
            cxx_name = "readerBackgroundColor",
            READ = reader_background_color,
            NOTIFY = reader_changed
        )]
        #[qproperty(
            QString,
            reader_text_color,
            cxx_name = "readerTextColor",
            READ = reader_text_color,
            NOTIFY = reader_changed
        )]
        #[qproperty(
            QString,
            reader_font_family,
            cxx_name = "readerFontFamily",
            READ = reader_font_family,
            NOTIFY = reader_changed
        )]
        #[qproperty(
            i32,
            reader_font_size,
            cxx_name = "readerFontSize",
            READ = reader_font_size,
            NOTIFY = reader_changed
        )]
        #[qproperty(
            i32,
            reader_line_height,
            cxx_name = "readerLineHeight",
            READ = reader_line_height,
            NOTIFY = reader_changed
        )]
        #[qproperty(
            i32,
            reader_column_width,
            cxx_name = "readerColumnWidth",
            READ = reader_column_width,
            NOTIFY = reader_changed
        )]
        #[qproperty(
            ReaderSettingsState,
            reader_settings_state,
            cxx_name = "readerSettingsState",
            READ = reader_settings_state,
            NOTIFY = reader_settings_changed
        )]
        #[qproperty(
            QString,
            reader_settings_error_text,
            cxx_name = "readerSettingsErrorText",
            READ = reader_settings_error_text,
            NOTIFY = reader_settings_changed
        )]
        #[qproperty(
            QString,
            reader_translation_language,
            cxx_name = "readerTranslationLanguage",
            READ = reader_translation_language,
            NOTIFY = reader_changed
        )]
        #[qproperty(
            QString,
            reader_text_translation_service,
            cxx_name = "readerTextTranslationService",
            READ = reader_text_translation_service,
            NOTIFY = reader_translation_options_changed
        )]
        #[qproperty(
            QString,
            reader_openrouter_model,
            cxx_name = "readerOpenRouterModel",
            READ = reader_openrouter_model,
            NOTIFY = reader_translation_options_changed
        )]
        #[qproperty(
            QString,
            reader_openrouter_effort,
            cxx_name = "readerOpenRouterEffort",
            READ = reader_openrouter_effort,
            NOTIFY = reader_translation_options_changed
        )]
        #[qproperty(
            bool,
            reader_openrouter_key_configured,
            cxx_name = "readerOpenRouterKeyConfigured",
            READ = reader_openrouter_key_configured,
            NOTIFY = reader_translation_options_changed
        )]
        #[qproperty(
            TranslationSettingsError,
            reader_translation_settings_error,
            cxx_name = "readerTranslationSettingsError",
            READ = reader_translation_settings_error,
            NOTIFY = reader_translation_options_changed
        )]
        #[qproperty(
            i32,
            reader_translation_model_count,
            cxx_name = "readerTranslationModelCount",
            READ = reader_translation_model_count,
            NOTIFY = reader_translation_options_changed
        )]
        #[qproperty(
            i32,
            reader_translation_effort_count,
            cxx_name = "readerTranslationEffortCount",
            READ = reader_translation_effort_count,
            NOTIFY = reader_translation_options_changed
        )]
        #[qproperty(
            QString,
            reader_translation_model,
            cxx_name = "readerTranslationModel",
            READ = reader_translation_model,
            NOTIFY = reader_translation_options_changed
        )]
        #[qproperty(
            QString,
            reader_translation_effort,
            cxx_name = "readerTranslationEffort",
            READ = reader_translation_effort,
            NOTIFY = reader_translation_options_changed
        )]
        #[qproperty(
            bool,
            reader_translation_options_loading,
            cxx_name = "readerTranslationOptionsLoading",
            READ = reader_translation_options_loading,
            NOTIFY = reader_translation_options_changed
        )]
        #[qproperty(
            bool,
            reader_translation_options_error,
            cxx_name = "readerTranslationOptionsError",
            READ = reader_translation_options_error,
            NOTIFY = reader_translation_options_changed
        )]
        #[qproperty(
            QString,
            chapter_query,
            cxx_name = "chapterQuery",
            READ = chapter_query,
            NOTIFY = reader_changed
        )]
        #[qproperty(ListMode, list_mode, cxx_name = "listMode", READ, NOTIFY)]
        #[qproperty(LibraryState, library_state, cxx_name = "libraryState", READ, NOTIFY)]
        #[qproperty(
            bool,
            library_pending,
            cxx_name = "libraryPending",
            READ = is_library_pending,
            NOTIFY
        )]
        #[qproperty(
            QString,
            library_error_text,
            cxx_name = "libraryErrorText",
            READ = library_error_text,
            NOTIFY
        )]
        #[qproperty(
            LibraryErrorKind,
            library_error_kind,
            cxx_name = "libraryErrorKind",
            READ = current_library_error_kind,
            NOTIFY = library_error_text_changed
        )]
        #[qproperty(bool, selected_favorite, cxx_name = "selectedFavorite", READ, NOTIFY)]
        #[qproperty(
            QString,
            selected_variant_id,
            cxx_name = "selectedVariantId",
            READ,
            NOTIFY
        )]
        #[qproperty(
            VariantSelectionState,
            selected_variant_state,
            cxx_name = "selectedVariantState",
            READ = variant_selection_state,
            NOTIFY
        )]
        #[qproperty(bool, busy, READ = is_searching, NOTIFY)]
        #[qproperty(bool, shared_importing, cxx_name = "sharedImporting", READ, NOTIFY)]
        #[qproperty(SearchState, search_state, cxx_name = "searchState", READ, NOTIFY)]
        #[qproperty(bool, inspecting, READ, NOTIFY)]
        #[qproperty(bool, resolving, READ, NOTIFY)]
        #[qproperty(QString, error_text, cxx_name = "errorText", READ = error_text, NOTIFY)]
        #[qproperty(QString, selected_work_id, cxx_name = "selectedWorkId", READ, NOTIFY)]
        #[qproperty(
            QString,
            selected_work_title,
            cxx_name = "selectedWorkTitle",
            READ,
            NOTIFY
        )]
        #[qproperty(
            QString,
            selected_work_description,
            cxx_name = "selectedWorkDescription",
            READ,
            NOTIFY
        )]
        #[qproperty(
            QString,
            selected_publication_id,
            cxx_name = "selectedPublicationId",
            READ = selected_publication_id,
            NOTIFY = selected_work_description_changed
        )]
        #[qproperty(
            QString,
            selected_publication_source,
            cxx_name = "selectedPublicationSource",
            READ = selected_publication_source,
            NOTIFY = selected_work_description_changed
        )]
        #[qproperty(
            QString,
            selected_work_category,
            cxx_name = "selectedWorkCategory",
            READ = selected_work_category,
            NOTIFY
        )]
        #[qproperty(
            QString,
            selected_variant_kind,
            cxx_name = "selectedVariantKind",
            READ = selected_variant_kind,
            NOTIFY = selected_variant_state_changed
        )]
        #[qproperty(
            bool,
            can_share_selected,
            cxx_name = "canShareSelected",
            READ = can_share_selected,
            NOTIFY = share_availability_changed
        )]
        #[qproperty(
            bool,
            selected_variant_saveable,
            cxx_name = "selectedVariantSaveable",
            READ = selected_variant_saveable,
            NOTIFY = selected_variant_state_changed
        )]
        type ContentController = super::ContentControllerRust;

        fn retranslate(self: Pin<&mut ContentController>);
        fn error_text(&self) -> QString;

        fn works_model(&self) -> *mut WorkModel;
        fn variants_model(&self) -> *mut VariantModel;
        fn downloads_model(&self) -> *mut DownloadModel;
        #[qinvokable]
        #[cxx_name = "initDownloads"]
        fn init_downloads(self: Pin<&mut Self>, destination: QString);
        #[qinvokable]
        #[cxx_name = "downloadSelected"]
        fn download_selected(self: Pin<&mut Self>, id: QString) -> bool;
        #[cxx_name = "canDownloadPlayback"]
        fn can_download_playback(&self, launch_token: u64) -> bool;
        #[cxx_name = "downloadPlayback"]
        fn download_playback(self: Pin<&mut Self>, launch_token: u64) -> bool;
        #[cxx_name = "downloadCaptured"]
        fn download_captured(
            self: Pin<&mut Self>,
            page: QString,
            title: QString,
            capture_id: QString,
        ) -> bool;
        #[cxx_name = "cancelCapturedDownload"]
        fn cancel_captured_download(self: Pin<&mut Self>);
        #[cxx_name = "watchCaptured"]
        fn watch_captured(
            self: Pin<&mut Self>,
            page: QString,
            title: QString,
            capture_id: QString,
        ) -> bool;
        #[cxx_name = "cancelCapturedWatch"]
        fn cancel_captured_watch(self: Pin<&mut Self>);
        #[qsignal]
        #[cxx_name = "capturedWatchFinished"]
        fn captured_watch_finished(self: Pin<&mut Self>, capture_id: QString, reason: QString);
        #[qsignal]
        #[cxx_name = "capturedDownloadFinished"]
        fn captured_download_finished(self: Pin<&mut Self>, capture_id: QString, reason: QString);
        #[qsignal]
        #[cxx_name = "playbackDownloadFinished"]
        fn playback_download_finished(self: Pin<&mut Self>, launch_token: u64, queued: bool);
        #[qsignal]
        #[cxx_name = "downloadQueued"]
        fn download_queued(self: Pin<&mut Self>);
        #[qsignal]
        #[cxx_name = "mediaToolsRequested"]
        fn media_tools_requested(self: Pin<&mut Self>, request_id: u64);
        #[qsignal]
        #[cxx_name = "mediaToolsCancelled"]
        fn media_tools_cancelled(self: Pin<&mut Self>, request_id: u64);
        #[cxx_name = "mediaToolsReady"]
        fn media_tools_ready(
            self: Pin<&mut Self>,
            request_id: u64,
            ffmpeg: QString,
            ffprobe: QString,
            error: QString,
        );
        #[qinvokable]
        #[cxx_name = "cancelDownload"]
        fn cancel_download(self: Pin<&mut Self>, id: QString) -> bool;
        #[qinvokable]
        #[cxx_name = "retryDownload"]
        fn retry_download(self: Pin<&mut Self>, id: QString) -> bool;
        fn sources_model(&self) -> *mut SourceModel;
        fn source_settings_state(&self) -> SettingsState;
        fn source_settings_error_text(&self) -> QString;
        #[qinvokable]
        #[cxx_name = "setSourceEnabled"]
        fn set_source_enabled(self: Pin<&mut Self>, id: QString, enabled: bool) -> bool;
        #[qinvokable]
        #[cxx_name = "retrySource"]
        fn retry_source(self: Pin<&mut Self>, id: QString) -> bool;
        #[qinvokable]
        #[cxx_name = "retrySourceSettings"]
        fn retry_source_settings(self: Pin<&mut Self>) -> bool;
        fn browser_choice(&self) -> QString;
        fn browser_connection_state(&self) -> ConnectionState;
        fn browser_connected_name(&self) -> QString;
        fn browser_setup_state(&self) -> SetupState;
        fn browser_setup_failure(&self) -> SetupFailure;
        fn browser_extension_directory(&self) -> QString;
        #[cxx_name = "setBrowserChoice"]
        fn set_browser_choice(self: Pin<&mut Self>, choice: QString);
        #[qinvokable]
        #[cxx_name = "chooseBrowser"]
        fn choose_browser(self: Pin<&mut Self>, choice: QString);
        #[qsignal]
        #[cxx_name = "browserChoiceRequested"]
        fn browser_choice_requested(self: Pin<&mut Self>, choice: QString);
        #[qinvokable]
        #[cxx_name = "watchBrowser"]
        fn watch_browser(self: Pin<&mut Self>);
        #[qinvokable]
        #[cxx_name = "closeBrowserSettings"]
        fn close_browser_settings(self: Pin<&mut Self>);
        #[qinvokable]
        #[cxx_name = "connectQcatch"]
        fn connect_qcatch(self: Pin<&mut Self>);
        #[qsignal]
        #[cxx_name = "browserChanged"]
        fn browser_changed(self: Pin<&mut Self>);
        #[qinvokable]
        #[cxx_name = "openSourceAccount"]
        fn open_source_account(self: Pin<&mut Self>, id: QString) -> bool;
        #[qinvokable]
        #[cxx_name = "openSourceSite"]
        fn open_source_site(self: Pin<&mut Self>, id: QString) -> bool;
        #[qsignal]
        #[cxx_name = "sourceAccountOpened"]
        fn source_account_opened(self: Pin<&mut Self>, id: QString, opened: bool);
        #[qsignal]
        #[cxx_name = "sourcesChanged"]
        fn sources_changed(self: Pin<&mut Self>);
        fn favorites_model(&self) -> *mut WorkModel;
        fn history_model(&self) -> *mut WorkModel;
        fn is_library_pending(&self) -> bool;
        fn library_error_text(&self) -> QString;
        fn current_library_error_kind(&self) -> LibraryErrorKind;
        fn variant_selection_state(&self) -> VariantSelectionState;
        fn selected_work_category(&self) -> QString;
        fn selected_publication_id(&self) -> QString;
        fn selected_publication_source(&self) -> QString;
        fn selected_variant_kind(&self) -> QString;
        fn can_share_selected(&self) -> bool;
        #[qsignal]
        #[cxx_name = "shareAvailabilityChanged"]
        fn share_availability_changed(self: Pin<&mut Self>);
        fn selected_variant_saveable(&self) -> bool;
        fn variant_episode_filter(&self) -> QString;
        fn variant_translation_filter(&self) -> QString;
        fn variant_source_filter(&self) -> QString;
        fn variant_player_filter(&self) -> QString;
        fn variant_language_filter(&self) -> QString;
        fn variant_transport_filter(&self) -> QString;
        fn variant_query(&self) -> QString;
        #[qsignal]
        #[cxx_name = "variantFiltersChanged"]
        fn variant_filters_changed(self: Pin<&mut Self>);
        #[qinvokable]
        #[cxx_name = "setVariantEpisodeFilter"]
        fn set_variant_episode_filter(self: Pin<&mut Self>, value: QString);
        #[qinvokable]
        #[cxx_name = "setVariantTranslationFilter"]
        fn set_variant_translation_filter(self: Pin<&mut Self>, value: QString);
        #[qinvokable]
        #[cxx_name = "setVariantProviderFilter"]
        fn set_variant_provider_filter(self: Pin<&mut Self>, source: QString, player: QString);
        #[qinvokable]
        #[cxx_name = "setVariantLanguageFilter"]
        fn set_variant_language_filter(self: Pin<&mut Self>, value: QString);
        #[qinvokable]
        #[cxx_name = "setVariantQuery"]
        fn set_variant_query(self: Pin<&mut Self>, value: QString);
        #[qinvokable]
        #[cxx_name = "clearVariantFilters"]
        fn clear_variant_filters(self: Pin<&mut Self>);
        fn reader_chapters(&self) -> *mut ChapterModel;
        fn reader_pages(&self) -> *mut PageModel;
        fn reader_text_blocks(&self) -> *mut TextBlockModel;
        fn reader_is_text(&self) -> bool;
        fn reader_text_generation(&self) -> u64;
        fn reader_text_anchor(&self) -> &QString;
        fn reader_text_offset(&self) -> f64;
        fn reader_translation_requested(&self) -> bool;
        fn reader_translation_busy(&self) -> bool;
        fn reader_translation_error(&self) -> bool;
        fn reader_translation_needs_codex(&self) -> bool;
        fn reader_translation_image_size_error(&self) -> bool;
        fn reader_chapter_index(&self) -> i32;
        fn reader_chapter_count(&self) -> i32;
        fn reader_mode(&self) -> ReaderMode;
        fn reader_image_width(&self) -> i32;
        fn reader_image_height(&self) -> i32;
        fn reader_open(&self) -> bool;
        fn reader_has_chapters(&self) -> bool;
        fn reader_busy(&self) -> bool;
        fn reader_error_text(&self) -> QString;
        fn reader_progress_error_text(&self) -> &QString;
        fn reader_progress_pending(&self) -> bool;
        fn reader_title(&self) -> &QString;
        fn reader_chapter_title(&self) -> &QString;
        fn reader_chapter_id(&self) -> &QString;
        fn reader_page_url(&self) -> QUrl;
        fn reader_page_index(&self) -> &i32;
        fn reader_page_count(&self) -> i32;
        fn reader_scroll_offset(&self) -> &f64;
        fn reader_fit(&self) -> ReaderFit;
        fn reader_background_color(&self) -> QString;
        fn reader_text_color(&self) -> QString;
        fn reader_font_family(&self) -> QString;
        fn reader_font_size(&self) -> i32;
        fn reader_line_height(&self) -> i32;
        fn reader_column_width(&self) -> i32;
        fn reader_settings_state(&self) -> ReaderSettingsState;
        fn reader_settings_error_text(&self) -> QString;
        fn reader_translation_language(&self) -> QString;
        fn reader_text_translation_service(&self) -> QString;
        fn reader_openrouter_model(&self) -> QString;
        fn reader_openrouter_effort(&self) -> QString;
        fn reader_openrouter_key_configured(&self) -> bool;
        fn reader_translation_settings_error(&self) -> TranslationSettingsError;
        fn reader_translation_model_count(&self) -> i32;
        fn reader_translation_effort_count(&self) -> i32;
        fn reader_translation_model(&self) -> QString;
        fn reader_translation_effort(&self) -> QString;
        fn reader_translation_options_loading(&self) -> bool;
        fn reader_translation_options_error(&self) -> bool;
        #[qinvokable]
        #[cxx_name = "readerTranslationModelAt"]
        fn reader_translation_model_at(&self, index: i32) -> QString;
        #[qinvokable]
        #[cxx_name = "readerTranslationEffortAt"]
        fn reader_translation_effort_at(&self, index: i32) -> QString;
        fn chapter_query(&self) -> &QString;

        #[qinvokable]
        #[cxx_name = "openReader"]
        fn open_reader(self: Pin<&mut Self>);
        #[qinvokable]
        #[cxx_name = "closeReader"]
        fn close_reader(self: Pin<&mut Self>);
        #[qinvokable]
        #[cxx_name = "selectChapter"]
        fn select_chapter(self: Pin<&mut Self>, id: QString);
        #[qinvokable]
        #[cxx_name = "turnReaderChapter"]
        fn turn_reader_chapter(self: Pin<&mut Self>, offset: i32);
        #[qinvokable]
        #[cxx_name = "selectReaderPage"]
        fn select_reader_page(self: Pin<&mut Self>, index: i32);
        #[qinvokable]
        #[cxx_name = "turnReaderPage"]
        fn turn_reader_page(self: Pin<&mut Self>, offset: i32);
        #[qinvokable]
        #[cxx_name = "setReaderFit"]
        fn set_reader_fit(self: Pin<&mut Self>, fit: ReaderFit) -> bool;
        #[qinvokable]
        #[cxx_name = "setReaderBackgroundColor"]
        fn set_reader_background_color(self: Pin<&mut Self>, color: QString) -> bool;
        #[qinvokable]
        #[cxx_name = "setReaderTextColor"]
        fn set_reader_text_color(self: Pin<&mut Self>, color: QString) -> bool;
        #[qinvokable]
        #[cxx_name = "setReaderFontFamily"]
        fn set_reader_font_family(self: Pin<&mut Self>, family: QString) -> bool;
        #[qinvokable]
        #[cxx_name = "setReaderFontSize"]
        fn set_reader_font_size(self: Pin<&mut Self>, size: i32) -> bool;
        #[qinvokable]
        #[cxx_name = "setReaderLineHeight"]
        fn set_reader_line_height(self: Pin<&mut Self>, height: i32) -> bool;
        #[qinvokable]
        #[cxx_name = "setReaderColumnWidth"]
        fn set_reader_column_width(self: Pin<&mut Self>, width: i32) -> bool;
        #[qinvokable]
        #[cxx_name = "resetReaderAppearance"]
        fn reset_reader_appearance(self: Pin<&mut Self>) -> bool;
        #[qinvokable]
        #[cxx_name = "retryReaderSettings"]
        fn retry_reader_settings(self: Pin<&mut Self>) -> bool;
        #[qinvokable]
        #[cxx_name = "setReaderTranslationLanguage"]
        fn set_reader_translation_language(self: Pin<&mut Self>, language: QString) -> bool;
        #[qinvokable]
        #[cxx_name = "loadReaderTranslationOptions"]
        fn load_reader_translation_options(self: Pin<&mut Self>) -> bool;
        #[qinvokable]
        #[cxx_name = "configureReaderTextTranslation"]
        fn configure_reader_text_translation(
            self: Pin<&mut Self>,
            service: QString,
            model: QString,
            effort: QString,
            api_key: QString,
            remove_key: bool,
        ) -> bool;
        #[qinvokable]
        #[cxx_name = "setReaderTranslationModel"]
        fn set_reader_translation_model(self: Pin<&mut Self>, model: QString) -> bool;
        #[qinvokable]
        #[cxx_name = "setReaderTranslationEffort"]
        fn set_reader_translation_effort(self: Pin<&mut Self>, effort: QString) -> bool;
        #[qsignal]
        #[cxx_name = "readerSettingsChanged"]
        fn reader_settings_changed(self: Pin<&mut Self>);
        #[qsignal]
        #[cxx_name = "readerTranslationOptionsChanged"]
        fn reader_translation_options_changed(self: Pin<&mut Self>);
        #[qinvokable]
        #[cxx_name = "setChapterQuery"]
        fn set_chapter_query(self: Pin<&mut Self>, query: QString);
        #[qinvokable]
        #[cxx_name = "retryReader"]
        fn retry_reader(self: Pin<&mut Self>);
        #[qinvokable]
        #[cxx_name = "readerPageShown"]
        fn reader_page_shown(self: Pin<&mut Self>, index: i32, url: QUrl);
        #[qinvokable]
        #[cxx_name = "readerPageFailed"]
        fn reader_page_failed(self: Pin<&mut Self>, index: i32, url: QUrl);
        #[qinvokable]
        #[cxx_name = "setReaderViewport"]
        fn set_reader_viewport(
            self: Pin<&mut Self>,
            generation: u64,
            first: i32,
            last: i32,
            anchor: i32,
            offset: f64,
            user_moved: bool,
        ) -> bool;
        #[qinvokable]
        #[cxx_name = "retryReaderPage"]
        fn retry_reader_page(self: Pin<&mut Self>, generation: u64, index: i32);
        #[qinvokable]
        #[cxx_name = "setReaderMode"]
        fn set_reader_mode(self: Pin<&mut Self>, mode: ReaderMode) -> bool;
        #[qinvokable]
        #[cxx_name = "scrollReaderPage"]
        fn scroll_reader_page(self: Pin<&mut Self>, url: QUrl, offset: f64);
        #[qinvokable]
        #[cxx_name = "retryReaderProgress"]
        fn retry_reader_progress(self: Pin<&mut Self>);
        #[qinvokable]
        #[cxx_name = "setReaderTextViewport"]
        fn set_reader_text_viewport(
            self: Pin<&mut Self>,
            generation: u64,
            block_id: QString,
            offset: f64,
            user_moved: bool,
        ) -> bool;
        #[qinvokable]
        #[cxx_name = "setReaderTextTranslation"]
        fn set_reader_text_translation(self: Pin<&mut Self>, translated: bool) -> bool;
        #[qinvokable]
        #[cxx_name = "setReaderMangaTranslation"]
        fn set_reader_manga_translation(self: Pin<&mut Self>, translated: bool) -> bool;
        #[qsignal]
        #[cxx_name = "readerTextPositionRequested"]
        fn reader_text_position_requested(
            self: Pin<&mut Self>,
            generation: u64,
            block_id: QString,
            offset: f64,
        );
        #[qsignal]
        #[cxx_name = "readerPositionRequested"]
        fn reader_position_requested(
            self: Pin<&mut Self>,
            generation: u64,
            index: i32,
            offset: f64,
        );
        #[qsignal]
        #[cxx_name = "readerChanged"]
        fn reader_changed(self: Pin<&mut Self>);
        #[qsignal]
        #[cxx_name = "readerAssetsCleared"]
        fn reader_assets_cleared(self: Pin<&mut Self>, generation: u64);
        #[qsignal]
        #[cxx_name = "readerAsset"]
        fn reader_asset(self: Pin<&mut Self>, generation: u64, id: QString, bytes: QByteArray);

        #[qinvokable]
        #[cxx_name = "initLibrary"]
        fn init_library(self: Pin<&mut Self>, profile_dir: QString);
        #[qinvokable]
        #[cxx_name = "retryLibrary"]
        fn retry_library(self: Pin<&mut Self>) -> bool;
        #[qinvokable]
        #[cxx_name = "refreshHistory"]
        fn refresh_history(self: Pin<&mut Self>) -> bool;
        #[qinvokable]
        #[cxx_name = "removeHistory"]
        fn remove_history(self: Pin<&mut Self>, id: QString) -> bool;
        #[qinvokable]
        #[cxx_name = "setListMode"]
        fn set_list_mode(self: Pin<&mut Self>, mode: ListMode);
        #[qinvokable]
        #[cxx_name = "setSelectedFavorite"]
        fn set_selected_favorite(self: Pin<&mut Self>, favorite: bool);
        #[qinvokable]
        #[cxx_name = "selectVariant"]
        fn select_variant(self: Pin<&mut Self>, id: QString);
        #[qinvokable]
        #[cxx_name = "shareSelected"]
        fn share_selected(self: Pin<&mut Self>) -> bool;
        #[cxx_name = "openSharedLink"]
        fn open_shared_link(self: Pin<&mut Self>, url: QString);
        #[qsignal]
        #[cxx_name = "sharedLinkFailed"]
        fn shared_link_failed(self: Pin<&mut Self>, malformed: bool);
        fn is_searching(&self) -> bool;
        #[cxx_name = "initializeBrowser"]
        fn initialize_browser(self: Pin<&mut Self>);
        #[cxx_name = "relayBrowserRequest"]
        fn relay_browser_request(
            self: Pin<&mut Self>,
            generation: u64,
            id: u64,
            request: QByteArray,
        );
        #[cxx_name = "cancelBrowserRequest"]
        fn cancel_browser_request(self: Pin<&mut Self>, generation: u64, id: u64);
        #[cxx_name = "closeBrowserSession"]
        fn close_browser_session(self: Pin<&mut Self>, generation: u64);
        #[cxx_name = "closeBrowserRelay"]
        fn close_browser_relay(self: Pin<&mut Self>);
        #[qsignal]
        #[cxx_name = "browserReply"]
        fn browser_reply(self: Pin<&mut Self>, generation: u64, id: u64, reply: QByteArray);

        #[qinvokable]
        fn search(self: Pin<&mut Self>, query: QString);
        #[qinvokable]
        #[cxx_name = "cancelSearch"]
        fn cancel_search(self: Pin<&mut Self>);
        #[qinvokable]
        #[cxx_name = "selectWork"]
        fn select_work(self: Pin<&mut Self>, id: QString);
        #[qinvokable]
        #[cxx_name = "importLocalBook"]
        fn import_local_book(self: Pin<&mut Self>, url: QUrl);
        fn local_book_error(&self) -> &QString;
        #[qsignal]
        fn local_book_error_changed(self: Pin<&mut Self>);
        #[qinvokable]
        #[cxx_name = "resolveVariant"]
        fn resolve_variant(self: Pin<&mut Self>, id: QString);
        #[qinvokable]
        #[cxx_name = "confirmPlayback"]
        fn confirm_playback(self: Pin<&mut Self>, token: u64) -> bool;
        #[qinvokable]
        #[cxx_name = "finishPlayback"]
        fn finish_playback(self: Pin<&mut Self>, token: u64) -> bool;
        #[qinvokable]
        #[cxx_name = "retryPlaybackHistory"]
        fn retry_playback_history(self: Pin<&mut Self>) -> bool;
        #[qsignal]
        #[cxx_name = "videoReady"]
        fn video_ready(
            self: Pin<&mut Self>,
            token: u64,
            identity: QString,
            url: QUrl,
            proxy_url: QUrl,
            title: QString,
            content_kind: QString,
            audio_url: QUrl,
            audio_title: QString,
            audio_type: QString,
            audio_language: QString,
            audio_codec: QString,
            audio_channels: i32,
            subtitles: QVector_QUrl,
        );

        #[qsignal]
        #[cxx_name = "torrentReady"]
        fn torrent_ready(self: Pin<&mut Self>, magnet: QString);

        #[qsignal]
        #[cxx_name = "torrentMetadataReady"]
        fn torrent_metadata_ready(self: Pin<&mut Self>, identity: QString, data: QByteArray);
    }

    impl cxx_qt::Threading for ContentController {}
}

#[derive(Default)]
struct Pending {
    generation: u64,
    task: Option<JoinHandle<()>>,
}

impl Pending {
    fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

pub struct Services {
    runtime: Runtime,
    catalog: Arc<catalog::Catalog>,
    language: String,
    reader_load: Arc<tokio::sync::Semaphore>,
    reader_bytes: Arc<tokio::sync::Semaphore>,
    retired_gateways: Vec<JoinHandle<()>>,
    retired_downloads: Vec<JoinHandle<()>>,
    browser: browser::BrowserService,
}

pub struct ContentControllerRust {
    works: UniquePtr<models::ffi::WorkModel>,
    favorites: UniquePtr<models::ffi::WorkModel>,
    history: UniquePtr<models::ffi::WorkModel>,
    variants: UniquePtr<models::ffi::VariantModel>,
    sources: UniquePtr<source_model::ffi::SourceModel>,
    downloads: UniquePtr<download_model::ffi::DownloadModel>,
    downloads_state: QString,
    download_queue: Option<Arc<qbutt_content::DownloadQueue<DownloadStore>>>,
    download_task: Option<JoinHandle<()>>,
    media_tool_requests: HashMap<u64, tokio::sync::oneshot::Sender<Result<MediaTools, String>>>,
    captured_media_tool_request: Option<u64>,
    captured_download: Pending,
    captured_watch: Pending,
    account_open: Pending,
    captured_intent: Option<CapturedIntent>,
    source_status: SourceState,
    list_mode: ffi::ListMode,

    search_state: ffi::SearchState,
    inspecting: bool,
    resolving: bool,
    error_kind: Option<ffi::ContentError>,
    selected_work_id: QString,
    selected_work_title: QString,
    selected_work_description: QString,
    selected_favorite: bool,
    selected_variant_id: QString,
    selected_group: Option<WorkGroup>,
    search_association: WorkAssociation,
    known_groups: HashMap<PublicationRef, WorkGroup>,
    work_preferences: HashMap<PublicationRef, VariantPreferences>,

    library_state: ffi::LibraryState,
    library_pending: usize,
    library_error_generation: u64,
    library_updates: VecDeque<LibraryUpdate>,
    publishing_library: bool,
    library_error_kind: ffi::LibraryErrorKind,
    library_worker: Option<LibraryWorker>,

    services: Option<Services>,
    search: Pending,
    inspection: Pending,
    inspection_attempted: HashSet<PublicationRef>,
    resolution: Pending,
    shared_import: Pending,
    shared_document: Option<SharedWork>,
    shared_works: Vec<Work>,
    shared_importing: bool,
    playback: PlaybackState,
    reader: ReaderState,
    local_book: Option<Arc<LocalBook>>,
    local_book_load: Pending,
    local_book_error: QString,
    reader_settings: ReaderSettingsState,
    translation_options: TranslationOptionsState,
    browser_requests: HashMap<(u64, u64), JoinHandle<()>>,
    browser_watch: Pending,
    browser_setup_generation: u64,
    browser_ui_active: bool,
    browser_choice: QString,
    browser_connection_state: ffi::ConnectionState,
    browser_connected_name: QString,
    browser_setup_state: ffi::SetupState,
    browser_setup_failure: ffi::SetupFailure,
    browser_extension_directory: QString,
}

impl Default for ContentControllerRust {
    fn default() -> Self {
        let browser = browser::BrowserService::new(std::path::PathBuf::from(
            ffi::browser_discovery_path().to_string(),
        ));
        let services = catalog::Catalog::new(browser.client())
            .map_err(|error| error.to_string())
            .and_then(|catalog| {
                Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .map_err(|error| error.to_string())
                    .map(|runtime| Services {
                        runtime,
                        catalog: Arc::new(catalog),
                        language: ffi::content_language().to_string(),
                        reader_load: Arc::new(tokio::sync::Semaphore::new(1)),
                        reader_bytes: Arc::new(tokio::sync::Semaphore::new(96 * 1024 * 1024)),
                        retired_gateways: Vec::new(),
                        retired_downloads: Vec::new(),
                        browser,
                    })
            });
        let services = match services {
            Ok(services) => Some(services),
            Err(error) => {
                ffi::log_content_error(&QString::from("initialize"), &QString::from(error));
                None
            }
        };
        let error_kind = services
            .is_none()
            .then_some(ffi::ContentError::ServicesUnavailable);

        let source_status = SourceState::default();
        let mut sources = source_model::ffi::create_source_model();
        sources.pin_mut().replace(source_status.rows());
        Self {
            works: models::ffi::create_work_model(),
            favorites: models::ffi::create_work_model(),
            history: models::ffi::create_work_model(),
            variants: models::ffi::create_variant_model(),
            sources,
            downloads: download_model::ffi::create_download_model(),
            downloads_state: QString::from("uninitialized"),
            download_queue: None,
            download_task: None,
            media_tool_requests: HashMap::new(),
            captured_media_tool_request: None,
            captured_download: Pending::default(),
            captured_watch: Pending::default(),
            account_open: Pending::default(),
            captured_intent: None,
            source_status,
            list_mode: ffi::ListMode::Search,

            search_state: ffi::SearchState::Idle,
            inspecting: false,
            resolving: false,
            error_kind,
            selected_work_id: QString::default(),
            selected_work_title: QString::default(),
            selected_work_description: QString::default(),
            selected_favorite: false,
            selected_variant_id: QString::default(),
            selected_group: None,
            search_association: WorkAssociation::default(),
            known_groups: HashMap::new(),
            work_preferences: HashMap::new(),

            library_state: ffi::LibraryState::Uninitialized,
            library_pending: 0,
            library_error_generation: 0,
            library_updates: VecDeque::new(),
            publishing_library: false,
            library_error_kind: ffi::LibraryErrorKind::None,
            library_worker: None,

            services,
            search: Pending::default(),
            inspection: Pending::default(),
            inspection_attempted: HashSet::new(),
            resolution: Pending::default(),
            shared_import: Pending::default(),
            shared_document: None,
            shared_works: Vec::new(),
            shared_importing: false,
            playback: PlaybackState::default(),
            reader: ReaderState::default(),
            local_book: None,
            local_book_load: Pending::default(),
            local_book_error: QString::default(),
            reader_settings: ReaderSettingsState::default(),
            translation_options: TranslationOptionsState::default(),
            browser_requests: HashMap::new(),
            browser_watch: Pending::default(),
            browser_setup_generation: 0,
            browser_ui_active: false,
            browser_choice: QString::from("auto"),
            browser_connection_state: ffi::ConnectionState::Checking,
            browser_connected_name: QString::default(),
            browser_setup_state: ffi::SetupState::Idle,
            browser_setup_failure: ffi::SetupFailure::None,
            browser_extension_directory: QString::default(),
        }
    }
}

impl Drop for ContentControllerRust {
    fn drop(&mut self) {
        self.search.cancel();
        self.inspection.cancel();
        self.resolution.cancel();
        self.shared_import.cancel();
        self.captured_download.cancel();
        self.captured_watch.cancel();
        self.account_open.cancel();
        self.browser_watch.cancel();
        self.source_status.cancel_retries();
        if let Some(task) = self.download_task.take() {
            task.abort();
        }
        if let Some(queue) = self.download_queue.take()
            && let Some(services) = &mut self.services
        {
            services
                .retired_downloads
                .push(services.runtime.spawn(async move { queue.close().await }));
        }
        self.reader.cancel();
        self.local_book_load.cancel();
        for (_, request) in self.browser_requests.drain() {
            request.abort();
        }
        if let Some(services) = &mut self.services {
            for (_, launch) in self.playback.launches.drain() {
                services.retire_gateway(launch.gateway);
            }
        }
        // Closing the sender drains accepted writes and closes SQLite on its worker.
        self.library_worker.take();
        if let Some(services) = self.services.take() {
            // Qt's pool drains the runtime and accepted blocking work after this
            // QObject closes, including cache retirement during application Exit.
            ffi::retire_content_services(Box::new(services));
        }
    }
}

impl ffi::ContentController {
    fn is_searching(&self) -> bool {
        self.search_state == ffi::SearchState::Searching
    }

    fn set_search_state(mut self: Pin<&mut Self>, value: ffi::SearchState) {
        if self.search_state == value {
            return;
        }
        let was_searching = self.is_searching();
        self.as_mut().rust_mut().search_state = value;
        if self.is_searching() != was_searching {
            self.as_mut().busy_changed();
        }
        if self.search_state == value {
            self.as_mut().search_state_changed();
        }
    }

    fn set_resolving(mut self: Pin<&mut Self>, value: bool) {
        if self.resolving != value {
            self.as_mut().rust_mut().resolving = value;
            self.as_mut().resolving_changed();
        }
    }

    pub fn retranslate(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().variants.pin_mut().retranslate();
        self.as_mut().rust_mut().sources.pin_mut().retranslate();
        self.as_mut().error_text_changed();
        self.as_mut().library_error_text_changed();
        self.as_mut().sources_changed();
    }

    fn error_text(&self) -> QString {
        self.error_kind.map(ffi::content_error).unwrap_or_default()
    }

    fn set_error(mut self: Pin<&mut Self>, value: Option<ffi::ContentError>) {
        if self.error_kind != value {
            self.as_mut().rust_mut().error_kind = value;
            self.as_mut().error_text_changed();
        }
    }

    fn works_model(&self) -> *mut models::ffi::WorkModel {
        self.works.as_ptr().cast_mut()
    }

    fn variants_model(&self) -> *mut models::ffi::VariantModel {
        self.variants.as_ptr().cast_mut()
    }
}
