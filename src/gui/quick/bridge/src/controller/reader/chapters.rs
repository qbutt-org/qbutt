use std::pin::Pin;
use std::sync::Arc;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use qbutt_content::SourceOperation;
use tokio::sync::watch;

use super::super::catalog::SourceResponse;
use super::super::ffi;
use super::super::reader_pages::read_chapter;
use super::ReaderViewport;

impl ffi::ContentController {
    pub(super) fn load_chapters(mut self: Pin<&mut Self>) {
        let Some(reference) = self.reader.reference.clone() else {
            return;
        };
        let Some(request) = self.source_request(SourceOperation::Chapters(reference)) else {
            return;
        };
        self.as_mut().rust_mut().reader.loading.cancel();
        let generation = self.reader.loading.generation;
        {
            let mut state = self.as_mut().rust_mut();
            state.reader.busy = true;
            state.reader.error = QString::default();
        }
        self.as_mut().reader_changed();
        if self.reader.loading.generation != generation || self.reader.reference.is_none() {
            return;
        }
        let qt_thread = self.qt_thread();
        let services = self.services.as_ref().unwrap();
        let catalog = Arc::clone(&services.catalog);
        let task = services.runtime.spawn(async move {
            let result = catalog.execute(&request.operation).await;
            let _ = qt_thread.queue(move |mut controller| {
                if controller.reader.loading.generation != generation
                    || controller.reader.reference.is_none()
                    || !controller.source_request_current(&request)
                {
                    return;
                }
                controller.as_mut().rust_mut().reader.loading.task = None;
                controller.as_mut().source_outcome(&request, &result);
                if controller.reader.loading.generation != generation
                    || controller.reader.reference.is_none()
                    || !controller.source_request_current(&request)
                {
                    return;
                }
                match result {
                    Ok(SourceResponse::Chapters(chapters)) => {
                        controller
                            .as_mut()
                            .rust_mut()
                            .reader
                            .chapters
                            .pin_mut()
                            .replace(chapters);
                        if controller.reader.loading.generation != generation
                            || controller.reader.reference.is_none()
                        {
                            return;
                        }
                        if !controller.reader.chapter_id.is_empty() {
                            return;
                        }
                        if let Some(progress) = controller.reader.resume.clone() {
                            if controller
                                .reader
                                .chapters
                                .find(&progress.chapter.id)
                                .is_some()
                            {
                                controller.load_chapter(
                                    QString::from(&progress.chapter.id),
                                    progress.page as usize,
                                    progress.scroll_offset,
                                );
                                return;
                            }
                            controller.as_mut().rust_mut().reader.error =
                                ffi::reader_error(ffi::ReaderError::ChapterUnavailable);
                        } else if let Some(first) = controller.reader.chapters.first() {
                            controller.select_chapter(QString::from(first.id));
                            return;
                        } else {
                            controller.as_mut().rust_mut().reader.error =
                                ffi::reader_error(ffi::ReaderError::ChapterUnavailable);
                        }
                    }
                    Err(error) => {
                        ffi::log_content_error(
                            &QString::from("chapters"),
                            &QString::from(error.to_string()),
                        );
                        controller.as_mut().rust_mut().reader.error =
                            ffi::reader_error(ffi::ReaderError::Chapters);
                    }
                    Ok(_) => unreachable!("chapters operation response"),
                }
                controller.as_mut().rust_mut().reader.busy = false;
                controller.as_mut().reader_changed();
            });
        });
        self.as_mut().rust_mut().reader.loading.task = Some(task);
    }

    pub fn select_chapter(self: Pin<&mut Self>, id: QString) {
        if self.reader.chapter_id == id && self.reader.error.is_empty() {
            return;
        }
        self.load_chapter(id, 0, 0.0);
    }

    pub(super) fn load_chapter(
        mut self: Pin<&mut Self>,
        id: QString,
        page: usize,
        scroll_offset: f64,
    ) {
        if self.reader.reference.is_none() {
            return;
        }
        let Some(chapter) = self.reader.chapters.find(&id.to_string()).cloned() else {
            return;
        };
        if Some(&chapter.reference.variant) != self.reader.reference.as_ref() {
            return;
        }
        let Some(request) =
            self.source_request(SourceOperation::Chapter(chapter.reference.clone()))
        else {
            return;
        };
        self.as_mut().rust_mut().reader.loading.cancel();
        self.as_mut().rust_mut().reader.chapter.cancel();
        let generation = self.reader.chapter.generation;
        let (sender, position) = watch::channel(ReaderViewport::page(page));
        {
            let mut state = self.as_mut().rust_mut();
            state.reader.chapter_id = id;
            state.reader.chapter_title = ffi::chapter_title(
                &QString::from(chapter.number.as_deref().unwrap_or_default()),
                &QString::from(chapter.volume.as_deref().unwrap_or_default()),
                &QString::from(chapter.title.as_deref().unwrap_or_default()),
            );
            state.reader.page_index = page as i32;
            state.reader.scroll_offset = scroll_offset;
            state.reader.shown_position = None;
            state.reader.resume = None;
            state.reader.error = QString::default();
            state.reader.busy = true;
            state.reader.position = Some(sender);
        }
        self.as_mut()
            .rust_mut()
            .reader
            .pages
            .pin_mut()
            .reset(generation, 0);
        if self.reader.reference.is_none() || self.reader.chapter.generation != generation {
            return;
        }
        self.as_mut().reader_changed();
        if self.reader.reference.is_none() || self.reader.chapter.generation != generation {
            return;
        }
        let qt_thread = self.qt_thread();
        let services = self.services.as_ref().unwrap();
        let Some(source) = services.reader_source(chapter.reference.variant.publication.source)
        else {
            return;
        };
        let load_limit = Arc::clone(&services.reader_load);
        let byte_limit = Arc::clone(&services.reader_bytes);
        let task = services.runtime.spawn(read_chapter(
            source, request, position, qt_thread, generation, load_limit, byte_limit,
        ));
        self.as_mut().rust_mut().reader.chapter.task = Some(task);
    }

    pub fn set_chapter_query(mut self: Pin<&mut Self>, query: QString) {
        if self.reader.query == query {
            return;
        }
        self.as_mut().rust_mut().reader.query = query.clone();
        self.as_mut()
            .rust_mut()
            .reader
            .chapters
            .pin_mut()
            .filter(&query.to_string());
        self.as_mut().reader_changed();
    }
}
