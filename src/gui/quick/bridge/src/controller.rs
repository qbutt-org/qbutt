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
    #[namespace = ""]
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    unsafe extern "C++" {
        include!("qbutt-quick/src/models.cxxqt.h");
        type WorkModel = crate::models::ffi::WorkModel;
        type VariantModel = crate::models::ffi::VariantModel;
        include!("qbutt-quick/native.h");
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
        #[qproperty(bool, busy, READ, NOTIFY)]
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
    busy: bool,
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
            busy: false,
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
    fn set_busy(mut self: Pin<&mut Self>, value: bool) {
        if self.busy != value {
            self.as_mut().rust_mut().busy = value;
            self.as_mut().busy_changed();
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

    fn set_selected_work_id(mut self: Pin<&mut Self>, value: QString) {
        if self.selected_work_id != value {
            self.as_mut().rust_mut().selected_work_id = value;
            self.as_mut().selected_work_id_changed();
        }
    }

    fn set_selected_work_title(mut self: Pin<&mut Self>, value: QString) {
        if self.selected_work_title != value {
            self.as_mut().rust_mut().selected_work_title = value;
            self.as_mut().selected_work_title_changed();
        }
    }

    fn set_selected_work_description(mut self: Pin<&mut Self>, value: QString) {
        if self.selected_work_description != value {
            self.as_mut().rust_mut().selected_work_description = value;
            self.as_mut().selected_work_description_changed();
        }
    }

    fn works_model(&self) -> *mut models::ffi::WorkModel {
        self.works.as_ptr().cast_mut()
    }

    fn variants_model(&self) -> *mut models::ffi::VariantModel {
        self.variants.as_ptr().cast_mut()
    }

    pub fn search(mut self: Pin<&mut Self>, query: QString) {
        self.as_mut().cancel_search();
        self.as_mut().cancel_detail();
        self.as_mut().rust_mut().works.pin_mut().clear();
        self.as_mut().rust_mut().variants.pin_mut().clear();
        self.as_mut().set_selected_work_id(QString::default());
        self.as_mut().set_selected_work_title(QString::default());
        self.as_mut()
            .set_selected_work_description(QString::default());
        self.as_mut().set_error_text(QString::default());

        let query = query.to_string().trim().to_owned();
        if query.is_empty() {
            return;
        }
        if self.services.is_none() {
            self.as_mut().set_error_text(ffi::content_error(0));
            return;
        }

        self.as_mut().set_busy(true);
        let generation = self.search.generation;
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
                match result {
                    Ok(works) => {
                        for work in works {
                            if controller.search.generation != generation {
                                return;
                            }
                            controller.as_mut().rust_mut().works.pin_mut().update(work);
                        }
                    }
                    Err(error) => {
                        ffi::log_content_error(
                            &QString::from("search"),
                            &QString::from(error.to_string()),
                        );
                        controller.as_mut().set_error_text(ffi::content_error(1));
                    }
                }
                if controller.search.generation == generation {
                    controller.as_mut().set_busy(false);
                }
            });
        });
        self.as_mut().rust_mut().search.task = Some(task);
    }

    pub fn cancel_search(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().search.cancel();
        self.as_mut().set_busy(false);
    }

    fn cancel_detail(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().inspection.cancel();
        self.as_mut().rust_mut().resolution.cancel();
        self.as_mut().set_inspecting(false);
        self.as_mut().set_resolving(false);
    }

    fn show_work(mut self: Pin<&mut Self>, work: &Work) {
        self.as_mut()
            .set_selected_work_id(QString::from(work_id(&work.reference)));
        self.as_mut()
            .set_selected_work_title(QString::from(&work.title));
        self.as_mut().set_selected_work_description(QString::from(
            work.description.as_deref().unwrap_or_default(),
        ));
        self.as_mut().rust_mut().variants.pin_mut().update(work);
    }

    pub fn select_work(mut self: Pin<&mut Self>, id: QString) {
        let Some(work) = self.works.find(&id.to_string()).cloned() else {
            return;
        };
        if self.selected_work_id == id && (self.inspecting || !work.needs_inspection) {
            return;
        }

        self.as_mut().cancel_detail();
        self.as_mut().set_error_text(QString::default());
        if self.selected_work_id != id {
            self.as_mut().rust_mut().variants.pin_mut().clear();
        }
        self.as_mut().show_work(&work);
        if !work.needs_inspection {
            return;
        }
        if self.services.is_none() {
            self.as_mut().set_error_text(ffi::content_error(0));
            return;
        }

        self.as_mut().set_inspecting(true);
        let generation = self.inspection.generation;
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
                        controller.as_mut().show_work(&work);
                        if controller.inspection.generation != generation {
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

        self.as_mut().set_resolving(true);
        self.as_mut().set_error_text(QString::default());
        self.as_mut().rust_mut().resolution.cancel();
        let generation = self.resolution.generation;
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
                controller.as_mut().set_resolving(false);
                if controller.resolution.generation != generation {
                    return;
                }
                match result {
                    Ok(ResolvedContent::Torrent {
                        reference: resolved,
                        magnet,
                        ..
                    }) if resolved == reference => {
                        controller
                            .as_mut()
                            .torrent_ready(QString::from(magnet.as_str()));
                    }
                    Err(error) => {
                        ffi::log_content_error(
                            &QString::from("resolve"),
                            &QString::from(error.to_string()),
                        );
                        controller.as_mut().set_error_text(ffi::content_error(3));
                    }
                    Ok(_) => {
                        ffi::log_content_error(
                            &QString::from("resolve"),
                            &QString::from("Resolved variant does not match the selected torrent"),
                        );
                        controller.as_mut().set_error_text(ffi::content_error(3));
                    }
                }
            });
        });
        self.as_mut().rust_mut().resolution.task = Some(task);
    }
}
