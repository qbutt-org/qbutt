use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QString, QUrl};
use qbutt_content::{Error, LibraryResult, LocalBook, PublicationRef, WorkGroup};

use super::ffi;
use super::library::LibraryAction;

impl ffi::ContentController {
    pub(super) fn local_book_error(&self) -> &QString {
        &self.local_book_error
    }

    fn set_local_book_error(mut self: Pin<&mut Self>, message: &str) {
        let message = QString::from(message);
        if self.local_book_error != message {
            self.as_mut().rust_mut().local_book_error = message;
            self.local_book_error_changed();
        }
    }

    pub fn import_local_book(mut self: Pin<&mut Self>, url: QUrl) {
        let Some(path) = url
            .to_local_file()
            .map(|value| PathBuf::from(value.to_string()))
        else {
            self.as_mut()
                .set_local_book_error("select");
            return;
        };
        if self.library_state != ffi::LibraryState::Ready || self.services.is_none() {
            self.as_mut()
                .set_local_book_error("library");
            return;
        }
        self.as_mut().set_local_book_error("");
        self.as_mut().rust_mut().local_book_load.cancel();
        let generation = self.local_book_load.generation;
        let qt_thread = self.qt_thread();
        let task =
            self.services
                .as_ref()
                .unwrap()
                .runtime
                .spawn_blocking(move || {
                    let result = LocalBook::open(path).map(Arc::new);
                    let _ = qt_thread.queue(move |mut controller| {
                        if controller.local_book_load.generation != generation {
                            return;
                        }
                        controller.as_mut().rust_mut().local_book_load.task = None;
                        match result {
                            Ok(book) => {
                                if !controller.as_mut().submit_library(
                                    LibraryAction::LocalBookImport { book, generation },
                                ) {
                                    controller.set_local_book_error(
                                        "save",
                                    );
                                }
                            }
                            Err(error) => {
                                ffi::log_content_error(
                                    &QString::from("local book import"),
                                    &QString::from(error.to_string()),
                                );
                                controller.set_local_book_error(local_book_error_code(&error));
                            }
                        }
                    });
                });
        self.as_mut().rust_mut().local_book_load.task = Some(task);
    }

    pub(super) fn load_local_book_from_library(
        mut self: Pin<&mut Self>,
        reference: PublicationRef,
    ) {
        self.as_mut().rust_mut().local_book_load.cancel();
        let generation = self.local_book_load.generation;
        self.as_mut().set_local_book_error("");
        if !self.as_mut().submit_library(LibraryAction::LocalBookPath {
            reference,
            generation,
        }) {
            self.as_mut()
                .set_local_book_error("library");
        }
    }

    pub(super) fn complete_local_book_import(
        mut self: Pin<&mut Self>,
        book: Arc<LocalBook>,
        result: LibraryResult<()>,
        generation: u64,
    ) {
        if self.local_book_load.generation != generation {
            return;
        }
        match result {
            Ok(()) => {
                self.as_mut().set_list_mode(ffi::ListMode::Search);
                self.as_mut().rust_mut().works.pin_mut().update(book.work());
                self.as_mut().show_local_book(book);
            }
            Err(error) => {
                ffi::log_content_error(
                    &QString::from("local book import"),
                    &QString::from(error.to_string()),
                );
                self.as_mut()
                    .set_local_book_error("save");
            }
        }
    }

    pub(super) fn complete_local_book_path(
        mut self: Pin<&mut Self>,
        reference: PublicationRef,
        result: LibraryResult<Option<PathBuf>>,
        generation: u64,
    ) {
        if self.local_book_load.generation != generation {
            return;
        }
        let path = match result {
            Ok(Some(path)) => path,
            Ok(None) => {
                self.as_mut()
                    .set_local_book_error("missing");
                return;
            }
            Err(error) => {
                ffi::log_content_error(
                    &QString::from("local book path"),
                    &QString::from(error.to_string()),
                );
                self.as_mut()
                    .set_local_book_error("read");
                return;
            }
        };
        let qt_thread = self.qt_thread();
        let task = self
            .services
            .as_ref()
            .unwrap()
            .runtime
            .spawn_blocking(move || {
                let result = LocalBook::open(path).and_then(|book| {
                    if book.reference() == reference {
                        Ok(Arc::new(book))
                    } else {
                        Err(qbutt_content::Error::InvalidLocalBook(
                            "The book file changed since import",
                        ))
                    }
                });
                let _ = qt_thread.queue(move |mut controller| {
                    if controller.local_book_load.generation != generation {
                        return;
                    }
                    controller.as_mut().rust_mut().local_book_load.task = None;
                    match result {
                        Ok(book) => controller.show_local_book(book),
                        Err(error) => {
                            ffi::log_content_error(
                                &QString::from("local book reopen"),
                                &QString::from(error.to_string()),
                            );
                            controller.set_local_book_error(local_book_error_code(&error));
                        }
                    }
                });
            });
        self.as_mut().rust_mut().local_book_load.task = Some(task);
    }

    fn show_local_book(mut self: Pin<&mut Self>, book: Arc<LocalBook>) {
        let (generation, _) = self.as_mut().cancel_detail();
        self.as_mut().rust_mut().local_book = Some(Arc::clone(&book));
        let group = WorkGroup::single(book.work());
        if !self.as_mut().show_group(&group, generation) {
            return;
        }
        self.as_mut().select_initial_variant();
        if self.inspection.generation == generation {
            self.open_reader();
        }
    }
}

fn local_book_error_code(error: &Error) -> &'static str {
    match error {
        Error::LocalBookIo(_) => "missing",
        Error::ResponseTooLarge => "large",
        Error::LocalBookChanged => "changed",
        _ => "invalid",
    }
}
