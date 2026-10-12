use std::collections::{HashMap, VecDeque};
use std::pin::Pin;
use std::sync::Arc;

use cxx::UniquePtr;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use qbutt_content::{
    AniLiberty, MangaDex, PublicationRef, ResolvedContent, RuTor, Source, VariantPreferences, Work,
};
use tokio::runtime::{Builder, Runtime};
use tokio::task::JoinHandle;

mod library;
mod reader;
mod reader_settings;
mod search;

use crate::models::{self, variant_id, work_id};
use library::{LibraryUpdate, LibraryWorker};
use reader::{ReaderState, RetiredReaderCache, clean_retired_reader_cache};
use reader_settings::ReaderSettingsState;

#[cxx_qt::bridge(namespace = "Qbutt")]
pub mod ffi {
    #[qml_element]
    qnamespace!("Qbutt::Search");
    #[qml_element]
    qnamespace!("Qbutt::Library");
    #[qml_element]
    qnamespace!("Qbutt::Reader");

    #[qenum]
    #[namespace = "Qbutt::Reader"]
    enum ReaderFit {
        Height,
        Width,
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
    }

    #[qenum]
    #[namespace = "Qbutt::Search"]
    enum VariantSelectionState {
        None,
        Inspecting,
        Available,
        Unavailable,
    }

    #[namespace = ""]
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qurl.h");
        type QUrl = cxx_qt_lib::QUrl;
    }

    unsafe extern "C++" {
        include!("qbutt_quick/src/models.cxxqt.h");
        type WorkModel = crate::models::ffi::WorkModel;
        type VariantModel = crate::models::ffi::VariantModel;
        include!("qbutt_quick/src/reader_models.cxxqt.h");
        type ChapterModel = crate::reader_models::ffi::ChapterModel;
        include!("qbutt_quick/native.h");
        #[rust_name = "content_error"]
        fn contentError(operation: i32) -> QString;
        #[rust_name = "log_content_error"]
        fn logContentError(operation: &QString, detail: &QString);
        #[rust_name = "content_language"]
        fn contentLanguage() -> QString;
        #[rust_name = "reader_error"]
        fn readerError(operation: i32) -> QString;
        #[rust_name = "chapter_title"]
        fn chapterTitle(number: &QString, volume: &QString, title: &QString) -> QString;
        #[rust_name = "validated_image_url"]
        fn validatedImageUrl(path: &QString) -> QString;
        #[rust_name = "retire_reader_cache"]
        fn retireReaderCache(cache: Box<RetiredReaderCache>);
        #[rust_name = "retire_content_services"]
        fn retireContentServices(services: Box<Services>);
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
        #[qproperty(*mut VariantModel, variants, READ = variants_model, CONSTANT)]
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
            READ,
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
        #[qproperty(SearchState, search_state, cxx_name = "searchState", READ, NOTIFY)]
        #[qproperty(bool, inspecting, READ, NOTIFY)]
        #[qproperty(bool, resolving, READ, NOTIFY)]
        #[qproperty(QString, error_text, cxx_name = "errorText", READ, NOTIFY)]
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
        type ContentController = super::ContentControllerRust;

        fn works_model(&self) -> *mut WorkModel;
        fn variants_model(&self) -> *mut VariantModel;
        fn favorites_model(&self) -> *mut WorkModel;
        fn history_model(&self) -> *mut WorkModel;
        fn is_library_pending(&self) -> bool;
        fn library_error_text(&self) -> QString;
        fn variant_selection_state(&self) -> VariantSelectionState;
        fn selected_work_category(&self) -> QString;
        fn selected_variant_kind(&self) -> QString;
        fn reader_chapters(&self) -> *mut ChapterModel;
        fn reader_open(&self) -> bool;
        fn reader_has_chapters(&self) -> bool;
        fn reader_busy(&self) -> bool;
        fn reader_error_text(&self) -> &QString;
        fn reader_progress_error_text(&self) -> &QString;
        fn reader_progress_pending(&self) -> bool;
        fn reader_title(&self) -> &QString;
        fn reader_chapter_title(&self) -> &QString;
        fn reader_chapter_id(&self) -> &QString;
        fn reader_page_url(&self) -> &QUrl;
        fn reader_page_index(&self) -> &i32;
        fn reader_page_count(&self) -> &i32;
        fn reader_scroll_offset(&self) -> &f64;
        fn reader_fit(&self) -> ReaderFit;
        fn reader_settings_state(&self) -> ReaderSettingsState;
        fn reader_settings_error_text(&self) -> QString;
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
        #[cxx_name = "selectReaderPage"]
        fn select_reader_page(self: Pin<&mut Self>, index: i32);
        #[qinvokable]
        #[cxx_name = "turnReaderPage"]
        fn turn_reader_page(self: Pin<&mut Self>, offset: i32);
        #[qinvokable]
        #[cxx_name = "setReaderFit"]
        fn set_reader_fit(self: Pin<&mut Self>, fit: ReaderFit) -> bool;
        #[qinvokable]
        #[cxx_name = "retryReaderSettings"]
        fn retry_reader_settings(self: Pin<&mut Self>) -> bool;
        #[qsignal]
        #[cxx_name = "readerSettingsChanged"]
        fn reader_settings_changed(self: Pin<&mut Self>);
        #[qinvokable]
        #[cxx_name = "setChapterQuery"]
        fn set_chapter_query(self: Pin<&mut Self>, query: QString);
        #[qinvokable]
        #[cxx_name = "retryReader"]
        fn retry_reader(self: Pin<&mut Self>);
        #[qinvokable]
        #[cxx_name = "readerPageShown"]
        fn reader_page_shown(self: Pin<&mut Self>, url: QUrl);
        #[qinvokable]
        #[cxx_name = "scrollReaderPage"]
        fn scroll_reader_page(self: Pin<&mut Self>, url: QUrl, offset: f64);
        #[qinvokable]
        #[cxx_name = "retryReaderProgress"]
        fn retry_reader_progress(self: Pin<&mut Self>);
        #[qsignal]
        #[cxx_name = "restoreReaderScroll"]
        fn restore_reader_scroll(self: Pin<&mut Self>, url: QUrl, offset: f64);
        #[qsignal]
        #[cxx_name = "readerChanged"]
        fn reader_changed(self: Pin<&mut Self>);

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
        fn is_searching(&self) -> bool;

        #[qinvokable]
        fn search(self: Pin<&mut Self>, query: QString);
        #[qinvokable]
        #[cxx_name = "cancelSearch"]
        fn cancel_search(self: Pin<&mut Self>);
        #[qinvokable]
        #[cxx_name = "selectWork"]
        fn select_work(self: Pin<&mut Self>, id: QString);
        #[qinvokable]
        #[cxx_name = "resolveVariant"]
        fn resolve_variant(self: Pin<&mut Self>, id: QString);

        #[qsignal]
        #[cxx_name = "torrentReady"]
        fn torrent_ready(self: Pin<&mut Self>, magnet: QString);
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
    anime: Arc<AniLiberty>,
    manga: Arc<MangaDex>,
    rutor: Arc<RuTor>,
    language: String,
}

pub struct ContentControllerRust {
    works: UniquePtr<models::ffi::WorkModel>,
    favorites: UniquePtr<models::ffi::WorkModel>,
    history: UniquePtr<models::ffi::WorkModel>,
    variants: UniquePtr<models::ffi::VariantModel>,
    list_mode: ffi::ListMode,

    search_state: ffi::SearchState,
    inspecting: bool,
    resolving: bool,
    error_text: QString,
    selected_work_id: QString,
    selected_work_title: QString,
    selected_work_description: QString,
    selected_favorite: bool,
    selected_variant_id: QString,
    selected_work: Option<Work>,
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
    resolution: Pending,
    reader: ReaderState,
    reader_settings: ReaderSettingsState,
}

impl Default for ContentControllerRust {
    fn default() -> Self {
        let services = AniLiberty::new()
            .map_err(|error| error.to_string())
            .and_then(|anime| {
                let manga = MangaDex::new().map_err(|error| error.to_string())?;
                let rutor = RuTor::new().map_err(|error| error.to_string())?;
                Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .map_err(|error| error.to_string())
                    .map(|runtime| Services {
                        runtime,
                        anime: Arc::new(anime),
                        manga: Arc::new(manga),
                        rutor: Arc::new(rutor),
                        language: ffi::content_language().to_string(),
                    })
            });
        let services = match services {
            Ok(services) => Some(services),
            Err(error) => {
                ffi::log_content_error(&QString::from("initialize"), &QString::from(error));
                None
            }
        };
        let error_text = if services.is_some() {
            QString::default()
        } else {
            ffi::content_error(0)
        };

        Self {
            works: models::ffi::create_work_model(),
            favorites: models::ffi::create_work_model(),
            history: models::ffi::create_work_model(),
            variants: models::ffi::create_variant_model(),
            list_mode: ffi::ListMode::Search,

            search_state: ffi::SearchState::Idle,
            inspecting: false,
            resolving: false,
            error_text,
            selected_work_id: QString::default(),
            selected_work_title: QString::default(),
            selected_work_description: QString::default(),
            selected_favorite: false,
            selected_variant_id: QString::default(),
            selected_work: None,
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
            resolution: Pending::default(),
            reader: ReaderState::default(),
            reader_settings: ReaderSettingsState::default(),
        }
    }
}

impl Drop for ContentControllerRust {
    fn drop(&mut self) {
        self.search.cancel();
        self.inspection.cancel();
        self.resolution.cancel();
        self.reader.cancel();
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

    fn set_inspecting(mut self: Pin<&mut Self>, value: bool) {
        if self.inspecting != value {
            self.as_mut().rust_mut().inspecting = value;
            self.as_mut().inspecting_changed();
            self.as_mut().selected_variant_state_changed();
        }
    }

    fn set_resolving(mut self: Pin<&mut Self>, value: bool) {
        if self.resolving != value {
            self.as_mut().rust_mut().resolving = value;
            self.as_mut().resolving_changed();
        }
    }

    fn set_error_text(mut self: Pin<&mut Self>, value: QString) {
        if self.error_text != value {
            self.as_mut().rust_mut().error_text = value;
            self.as_mut().error_text_changed();
        }
    }

    fn works_model(&self) -> *mut models::ffi::WorkModel {
        self.works.as_ptr().cast_mut()
    }

    fn variants_model(&self) -> *mut models::ffi::VariantModel {
        self.variants.as_ptr().cast_mut()
    }

    fn selected_work_category(&self) -> QString {
        let category = self
            .selected_work
            .as_ref()
            .map(|work| models::category_key(work.category))
            .unwrap_or_default();
        QString::from(category)
    }

    fn selected_variant_kind(&self) -> QString {
        let transport = self
            .variants
            .find(&self.selected_variant_id.to_string())
            .map(|variant| variant.transport);
        QString::from(match transport {
            Some(qbutt_content::Transport::Torrent) => "torrent",
            Some(qbutt_content::Transport::Video) => "video",
            Some(qbutt_content::Transport::Manga) => "manga",
            None => "",
        })
    }

    fn cancel_detail(mut self: Pin<&mut Self>) -> (u64, u64) {
        let (generations, inspecting_changed, resolving_changed) = {
            let mut state = self.as_mut().rust_mut();
            state.inspection.cancel();
            state.resolution.cancel();
            let changed = (
                (state.inspection.generation, state.resolution.generation),
                state.inspecting,
                state.resolving,
            );
            state.inspecting = false;
            state.resolving = false;
            changed
        };
        self.as_mut().close_reader();
        if inspecting_changed {
            self.as_mut().inspecting_changed();
            self.as_mut().selected_variant_state_changed();
        }
        if resolving_changed {
            self.as_mut().resolving_changed();
        }
        generations
    }

    fn set_selection(mut self: Pin<&mut Self>, work: Option<&Work>, generation: u64) -> bool {
        if self.inspection.generation != generation {
            return false;
        }
        let (id, title, description) = match work {
            Some(work) => (
                QString::from(work_id(&work.reference)),
                QString::from(&work.title),
                QString::from(work.description.as_deref().unwrap_or_default()),
            ),
            None => (QString::default(), QString::default(), QString::default()),
        };
        let variant_id = work
            .and_then(|work| self.work_preferences.get(&work.reference))
            .and_then(|preferences| preferences.selected.as_ref())
            .map(variant_id)
            .map(QString::from)
            .unwrap_or_default();
        let (id_changed, category_changed, title_changed, description_changed, variant_changed) = {
            let mut state = self.as_mut().rust_mut();
            let changed = (
                state.selected_work_id != id,
                state.selected_work.as_ref().map(|work| work.category)
                    != work.map(|work| work.category),
                state.selected_work_title != title,
                state.selected_work_description != description,
                state.selected_variant_id != variant_id,
            );
            state.selected_work_id = id;
            state.selected_work_title = title;
            state.selected_work_description = description;
            state.selected_work = work.cloned();
            state.selected_variant_id = variant_id;
            changed
        };
        if id_changed {
            self.as_mut().selected_work_id_changed();
        }
        if self.inspection.generation != generation {
            return false;
        }
        if category_changed {
            self.as_mut().selected_work_category_changed();
        }
        if self.inspection.generation != generation {
            return false;
        }
        if title_changed {
            self.as_mut().selected_work_title_changed();
        }
        if self.inspection.generation != generation {
            return false;
        }
        if description_changed {
            self.as_mut().selected_work_description_changed();
        }
        if self.inspection.generation != generation {
            return false;
        }
        if variant_changed {
            self.as_mut().selected_variant_id_changed();
        }
        if self.inspection.generation != generation {
            return false;
        }
        self.as_mut().refresh_selected_favorite();
        self.inspection.generation == generation
    }

    fn show_work(mut self: Pin<&mut Self>, work: &Work, generation: u64) -> bool {
        if !self.as_mut().set_selection(Some(work), generation) {
            return false;
        }
        self.as_mut().rust_mut().variants.pin_mut().update(work);
        if self.inspection.generation != generation {
            return false;
        }
        self.as_mut().selected_variant_state_changed();
        self.inspection.generation == generation
    }

    pub fn select_work(mut self: Pin<&mut Self>, id: QString) {
        let model = match self.list_mode {
            ffi::ListMode::Favorites => &self.favorites,
            ffi::ListMode::History => &self.history,
            _ => &self.works,
        };
        let Some(mut work) = model.find(&id.to_string()).cloned() else {
            return;
        };
        if self.list_mode != ffi::ListMode::Search {
            work.needs_inspection = true;
        }
        if self.selected_work_id == id && (self.inspecting || !work.needs_inspection) {
            return;
        }

        let (generation, resolution_generation) = self.as_mut().cancel_detail();
        if self.inspection.generation != generation
            || self.resolution.generation != resolution_generation
        {
            return;
        }
        if self.selected_work_id != id {
            self.as_mut().rust_mut().variants.pin_mut().clear();
            if self.inspection.generation != generation
                || self.resolution.generation != resolution_generation
            {
                return;
            }
        }
        let should_inspect = work.needs_inspection && self.services.is_some();
        // Cached variants cannot become actionable during synchronous selection notifications.
        self.as_mut().rust_mut().inspecting = should_inspect;
        if !self.as_mut().show_work(&work, generation) {
            return;
        }
        self.as_mut().set_error_text(QString::default());
        if self.inspection.generation != generation {
            return;
        }
        if should_inspect {
            self.as_mut().inspecting_changed();
        }
        if self.inspection.generation != generation {
            return;
        }
        if !work.needs_inspection {
            self.as_mut().select_initial_variant();
            return;
        }
        if self.services.is_none() {
            self.as_mut().set_error_text(ffi::content_error(0));
            return;
        }

        let qt_thread = self.qt_thread();
        let services = self.services.as_ref().unwrap();
        let anime = Arc::clone(&services.anime);
        let manga = Arc::clone(&services.manga);
        let rutor = Arc::clone(&services.rutor);
        let language = services.language.clone();
        let task = services.runtime.spawn(async move {
            let result = match work.reference.source {
                Source::AniLiberty => anime.inspect(&work.reference).await,
                Source::MangaDex => manga.inspect(&work.reference, &language).await,
                Source::RuTor => rutor.inspect(&work.reference).await,
            };
            let _ = qt_thread.queue(move |mut controller| {
                if controller.inspection.generation != generation {
                    return;
                }
                controller.as_mut().rust_mut().inspection.task = None;
                match result {
                    Ok(work) => {
                        if !controller.as_mut().show_work(&work, generation) {
                            return;
                        }
                        if controller.works.find(&work_id(&work.reference)).is_some() {
                            controller.as_mut().rust_mut().works.pin_mut().update(work);
                        }
                    }
                    Err(error) => {
                        ffi::log_content_error(
                            &QString::from("inspect"),
                            &QString::from(error.to_string()),
                        );
                        controller.as_mut().set_error_text(ffi::content_error(2));
                    }
                }
                if controller.inspection.generation == generation {
                    controller.as_mut().set_inspecting(false);
                    if controller.inspection.generation == generation
                        && controller.error_text.is_empty()
                    {
                        if controller.selected_variant_id.is_empty() {
                            controller.as_mut().select_initial_variant();
                        } else {
                            controller.as_mut().save_selected_preferences();
                        }
                    }
                }
            });
        });
        self.as_mut().rust_mut().inspection.task = Some(task);
    }

    pub fn resolve_variant(mut self: Pin<&mut Self>, id: QString) {
        if self.resolving || self.inspecting {
            return;
        }
        let Some(variant) = self.variants.find(&id.to_string()) else {
            return;
        };
        let reference = variant.reference.clone();
        let selection_generation = self.inspection.generation;
        self.as_mut().select_variant(id.clone());
        if self.inspection.generation != selection_generation || self.selected_variant_id != id {
            return;
        }
        if self.services.is_none() {
            self.as_mut().set_error_text(ffi::content_error(0));
            return;
        }

        self.as_mut().rust_mut().resolution.cancel();
        let generation = self.resolution.generation;
        self.as_mut().set_error_text(QString::default());
        if self.resolution.generation != generation {
            return;
        }
        self.as_mut().set_resolving(true);
        if self.resolution.generation != generation {
            return;
        }
        let qt_thread = self.qt_thread();
        let services = self.services.as_ref().unwrap();
        let anime = Arc::clone(&services.anime);
        let rutor = Arc::clone(&services.rutor);
        let task = services.runtime.spawn(async move {
            let result = match reference.publication.source {
                Source::AniLiberty => anime.resolve(&reference).await,
                Source::RuTor => rutor.resolve(&reference).await,
                Source::MangaDex => Err(qbutt_content::Error::InvalidReference),
            };
            let _ = qt_thread.queue(move |mut controller| {
                if controller.resolution.generation != generation {
                    return;
                }
                controller.as_mut().rust_mut().resolution.task = None;
                let magnet = match result {
                    Ok(ResolvedContent::Torrent {
                        reference: resolved,
                        magnet,
                        ..
                    }) if resolved == reference => Some(QString::from(magnet.as_str())),
                    Err(error) => {
                        ffi::log_content_error(
                            &QString::from("resolve"),
                            &QString::from(error.to_string()),
                        );
                        controller.as_mut().set_error_text(ffi::content_error(3));
                        None
                    }
                    Ok(_) => {
                        ffi::log_content_error(
                            &QString::from("resolve"),
                            &QString::from("Resolved variant does not match the selected torrent"),
                        );
                        controller.as_mut().set_error_text(ffi::content_error(3));
                        None
                    }
                };
                if controller.resolution.generation != generation {
                    return;
                }
                controller.as_mut().set_resolving(false);
                if controller.resolution.generation == generation
                    && let Some(magnet) = magnet
                {
                    controller.as_mut().torrent_ready(magnet);
                }
            });
        });
        self.as_mut().rust_mut().resolution.task = Some(task);
    }
}
