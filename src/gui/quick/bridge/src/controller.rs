use std::pin::Pin;
use std::sync::Arc;

use cxx::UniquePtr;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use qbutt_content::{AniLiberty, ResolvedContent, Work};
use tokio::runtime::{Builder, Runtime};
use tokio::task::JoinHandle;

use crate::models::{self, work_id};

#[cxx_qt::bridge(namespace = "Qbutt")]
pub mod ffi {
    #[qml_element]
    qnamespace!("Qbutt::Search");

    #[qenum]
    #[namespace = "Qbutt::Search"]
    enum SearchState {
        Idle,
        Searching,
        Succeeded,
        Cancelled,
        Failed,
    }

    #[namespace = ""]
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    unsafe extern "C++" {
        include!("qbutt_quick/src/models.cxxqt.h");
        type WorkModel = crate::models::ffi::WorkModel;
        type VariantModel = crate::models::ffi::VariantModel;
        include!("qbutt_quick/native.h");
        #[rust_name = "content_error"]
        fn contentError(operation: i32) -> QString;
        #[rust_name = "log_content_error"]
        fn logContentError(operation: &QString, detail: &QString);
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(*mut WorkModel, works, READ = works_model, CONSTANT)]
        #[qproperty(*mut VariantModel, variants, READ = variants_model, CONSTANT)]
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
        type ContentController = super::ContentControllerRust;

        fn works_model(&self) -> *mut WorkModel;
        fn variants_model(&self) -> *mut VariantModel;
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

struct Services {
    runtime: Runtime,
    source: Arc<AniLiberty>,
}

pub struct ContentControllerRust {
    works: UniquePtr<models::ffi::WorkModel>,
    variants: UniquePtr<models::ffi::VariantModel>,
    search_state: ffi::SearchState,
    inspecting: bool,
    resolving: bool,
    error_text: QString,
    selected_work_id: QString,
    selected_work_title: QString,
    selected_work_description: QString,
    services: Option<Services>,
    search: Pending,
    inspection: Pending,
    resolution: Pending,
}

impl Default for ContentControllerRust {
    fn default() -> Self {
        let services = AniLiberty::new()
            .map_err(|error| error.to_string())
            .and_then(|source| {
                Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .map_err(|error| error.to_string())
                    .map(|runtime| Services {
                        runtime,
                        source: Arc::new(source),
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
            variants: models::ffi::create_variant_model(),
            search_state: ffi::SearchState::Idle,
            inspecting: false,
            resolving: false,
            error_text,
            selected_work_id: QString::default(),
            selected_work_title: QString::default(),
            selected_work_description: QString::default(),
            services,
            search: Pending::default(),
            inspection: Pending::default(),
            resolution: Pending::default(),
        }
    }
}

impl Drop for ContentControllerRust {
    fn drop(&mut self) {
        self.search.cancel();
        self.inspection.cancel();
        self.resolution.cancel();
        if let Some(services) = self.services.take() {
            // QObject destruction must not wait for network work on the GUI thread.
            services.runtime.shutdown_background();
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

    pub fn search(mut self: Pin<&mut Self>, query: QString) {
        let query = query.to_string().trim().to_owned();
        self.as_mut().rust_mut().search.cancel();
        let generation = self.search.generation;
        let previous_selection_generation = self.inspection.generation;
        let previous_resolution_generation = self.resolution.generation;
        self.as_mut().set_search_state(ffi::SearchState::Idle);
        if self.search.generation != generation
            || self.inspection.generation != previous_selection_generation
            || self.resolution.generation != previous_resolution_generation
        {
            return;
        }
        let (selection_generation, resolution_generation) = self.as_mut().cancel_detail();
        if self.search.generation != generation
            || self.inspection.generation != selection_generation
            || self.resolution.generation != resolution_generation
        {
            return;
        }

        self.as_mut().rust_mut().works.pin_mut().clear();
        if self.search.generation != generation
            || self.inspection.generation != selection_generation
            || self.resolution.generation != resolution_generation
        {
            return;
        }
        self.as_mut().rust_mut().variants.pin_mut().clear();
        if self.search.generation != generation
            || self.inspection.generation != selection_generation
            || self.resolution.generation != resolution_generation
        {
            return;
        }
        if !self.as_mut().set_selection(None, selection_generation)
            || self.search.generation != generation
        {
            return;
        }
        self.as_mut().set_error_text(QString::default());
        if self.search.generation != generation {
            return;
        }
        if query.is_empty() {
            return;
        }
        if self.services.is_none() {
            self.as_mut().set_error_text(ffi::content_error(0));
            if self.search.generation == generation {
                self.as_mut().set_search_state(ffi::SearchState::Failed);
            }
            return;
        }
        self.as_mut().set_search_state(ffi::SearchState::Searching);
        if self.search.generation != generation {
            return;
        }

        let qt_thread = self.qt_thread();
        let services = self.services.as_ref().unwrap();
        let source = Arc::clone(&services.source);
        let task = services.runtime.spawn(async move {
            let result = source.search(&query).await;
            let _ = qt_thread.queue(move |mut controller| {
                if controller.search.generation != generation {
                    return;
                }
                controller.as_mut().rust_mut().search.task = None;
                let state = match result {
                    Ok(works) => {
                        for work in works {
                            if controller.search.generation != generation {
                                return;
                            }
                            controller.as_mut().rust_mut().works.pin_mut().update(work);
                        }
                        ffi::SearchState::Succeeded
                    }
                    Err(error) => {
                        ffi::log_content_error(
                            &QString::from("search"),
                            &QString::from(error.to_string()),
                        );
                        controller.as_mut().set_error_text(ffi::content_error(1));
                        ffi::SearchState::Failed
                    }
                };
                if controller.search.generation == generation {
                    controller.as_mut().set_search_state(state);
                }
            });
        });
        self.as_mut().rust_mut().search.task = Some(task);
    }

    pub fn cancel_search(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().search.cancel();
        if self.is_searching() {
            self.as_mut().set_search_state(ffi::SearchState::Cancelled);
        }
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
        if inspecting_changed {
            self.as_mut().inspecting_changed();
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
        let (id_changed, title_changed, description_changed) = {
            let mut state = self.as_mut().rust_mut();
            let changed = (
                state.selected_work_id != id,
                state.selected_work_title != title,
                state.selected_work_description != description,
            );
            state.selected_work_id = id;
            state.selected_work_title = title;
            state.selected_work_description = description;
            changed
        };
        if id_changed {
            self.as_mut().selected_work_id_changed();
        }
        if title_changed {
            self.as_mut().selected_work_title_changed();
        }
        if description_changed {
            self.as_mut().selected_work_description_changed();
        }
        self.inspection.generation == generation
    }

    fn show_work(mut self: Pin<&mut Self>, work: &Work, generation: u64) -> bool {
        if !self.as_mut().set_selection(Some(work), generation) {
            return false;
        }
        self.as_mut().rust_mut().variants.pin_mut().update(work);
        self.inspection.generation == generation
    }

    pub fn select_work(mut self: Pin<&mut Self>, id: QString) {
        let Some(work) = self.works.find(&id.to_string()).cloned() else {
            return;
        };
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
        if !self.as_mut().show_work(&work, generation) {
            return;
        }
        self.as_mut().set_error_text(QString::default());
        if self.inspection.generation != generation {
            return;
        }
        let should_inspect = work.needs_inspection && self.services.is_some();
        self.as_mut().set_inspecting(should_inspect);
        if self.inspection.generation != generation {
            return;
        }
        if !work.needs_inspection {
            return;
        }
        if self.services.is_none() {
            self.as_mut().set_error_text(ffi::content_error(0));
            return;
        }

        let qt_thread = self.qt_thread();
        let services = self.services.as_ref().unwrap();
        let source = Arc::clone(&services.source);
        let task = services.runtime.spawn(async move {
            let result = source.inspect(&work.reference).await;
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
                        controller.as_mut().rust_mut().works.pin_mut().update(work);
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
                }
            });
        });
        self.as_mut().rust_mut().inspection.task = Some(task);
    }

    pub fn resolve_variant(mut self: Pin<&mut Self>, id: QString) {
        if self.resolving {
            return;
        }
        let Some(variant) = self.variants.find(&id.to_string()) else {
            return;
        };
        let reference = variant.reference.clone();
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
        let source = Arc::clone(&services.source);
        let task = services.runtime.spawn(async move {
            let result = source.resolve(&reference).await;
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
