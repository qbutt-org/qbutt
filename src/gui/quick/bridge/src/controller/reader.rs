use std::collections::BTreeMap;
use std::io::Write;
use std::pin::Pin;
use std::sync::Arc;

use cxx::UniquePtr;
use cxx_qt::{CxxQtThread, CxxQtType, Threading};
use cxx_qt_lib::QString;
use qbutt_content::{ChapterPages, ChapterRef, MangaDex, Transport, VariantRef};
use tempfile::{NamedTempFile, TempDir, TempPath};
use tokio::sync::watch;

use super::{Pending, ffi};
use crate::reader_models;

pub(super) struct ReaderState {
    chapters: UniquePtr<reader_models::ffi::ChapterModel>,
    reference: Option<VariantRef>,
    open: bool,
    busy: bool,
    error: QString,
    chapter_title: QString,
    chapter_id: QString,
    page_url: QString,
    page_index: i32,
    page_count: i32,
    fit: ffi::ReaderFit,
    query: QString,
    loading: Pending,
    chapter: Pending,
    position: Option<watch::Sender<usize>>,
}

impl Default for ReaderState {
    fn default() -> Self {
        Self {
            chapters: reader_models::ffi::create_chapter_model(),
            reference: None,
            open: false,
            busy: false,
            error: QString::default(),
            chapter_title: QString::default(),
            chapter_id: QString::default(),
            page_url: QString::default(),
            page_index: 0,
            page_count: 0,
            fit: ffi::ReaderFit::Height,
            query: QString::default(),
            loading: Pending::default(),
            chapter: Pending::default(),
            position: None,
        }
    }
}

impl ReaderState {
    pub(super) fn cancel(&mut self) {
        self.loading.cancel();
        self.chapter.cancel();
        self.position.take();
    }
}

impl ffi::ContentController {
    pub(super) fn reader_chapters(&self) -> *mut reader_models::ffi::ChapterModel {
        self.reader.chapters.as_ptr().cast_mut()
    }

    pub(super) fn reader_open(&self) -> bool {
        self.reader.open
    }

    pub(super) fn reader_busy(&self) -> bool {
        self.reader.busy
    }

    pub(super) fn reader_error_text(&self) -> &QString {
        &self.reader.error
    }

    pub(super) fn reader_title(&self) -> &QString {
        &self.selected_work_title
    }

    pub(super) fn reader_chapter_title(&self) -> &QString {
        &self.reader.chapter_title
    }

    pub(super) fn reader_chapter_id(&self) -> &QString {
        &self.reader.chapter_id
    }

    pub(super) fn reader_page_url(&self) -> &QString {
        &self.reader.page_url
    }

    pub(super) fn reader_page_index(&self) -> &i32 {
        &self.reader.page_index
    }

    pub(super) fn reader_page_count(&self) -> &i32 {
        &self.reader.page_count
    }

    pub(super) fn reader_fit(&self) -> &ffi::ReaderFit {
        &self.reader.fit
    }

    pub(super) fn chapter_query(&self) -> &QString {
        &self.reader.query
    }

    pub fn open_reader(mut self: Pin<&mut Self>) {
        if self.reader.open || self.inspecting {
            return;
        }
        let Some(variant) = self.variants.find(&self.selected_variant_id.to_string()) else {
            return;
        };
        if variant.transport != Transport::Manga || self.services.is_none() {
            return;
        }
        let reference = variant.reference.clone();
        {
            let mut state = self.as_mut().rust_mut();
            state.reader.cancel();
            state.reader.reference = Some(reference);
            state.reader.open = true;
            state.reader.chapter_id = QString::default();
            state.reader.chapter_title = QString::default();
            state.reader.page_url = QString::default();
            state.reader.page_count = 0;
            state.reader.page_index = 0;
        }
        self.as_mut().load_chapters();
    }

    pub fn close_reader(mut self: Pin<&mut Self>) {
        if !self.reader.open {
            return;
        }
        {
            let mut state = self.as_mut().rust_mut();
            state.reader.cancel();
            state.reader.open = false;
            state.reader.busy = false;
            state.reader.error = QString::default();
            state.reader.page_url = QString::default();
            state.reader.reference = None;
        }
        self.as_mut().reader_changed();
    }

    fn load_chapters(mut self: Pin<&mut Self>) {
        let Some(reference) = self.reader.reference.clone() else {
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
        if self.reader.loading.generation != generation || !self.reader.open {
            return;
        }
        let qt_thread = self.qt_thread();
        let services = self.services.as_ref().unwrap();
        let manga = Arc::clone(&services.manga);
        let task = services.runtime.spawn(async move {
            let result = manga.chapters(&reference).await;
            let _ = qt_thread.queue(move |mut controller| {
                if controller.reader.loading.generation != generation || !controller.reader.open {
                    return;
                }
                controller.as_mut().rust_mut().reader.loading.task = None;
                match result {
                    Ok(chapters) => {
                        controller
                            .as_mut()
                            .rust_mut()
                            .reader
                            .chapters
                            .pin_mut()
                            .replace(chapters);
                        if controller.reader.loading.generation != generation
                            || !controller.reader.open
                        {
                            return;
                        }
                        if !controller.reader.chapter_id.is_empty() {
                            return;
                        }
                        if let Some(first) = controller.reader.chapters.first() {
                            controller.select_chapter(QString::from(first.id));
                            return;
                        }
                        controller.as_mut().rust_mut().reader.error = ffi::reader_error(3);
                    }
                    Err(error) => {
                        ffi::log_content_error(
                            &QString::from("chapters"),
                            &QString::from(error.to_string()),
                        );
                        controller.as_mut().rust_mut().reader.error = ffi::reader_error(0);
                    }
                }
                controller.as_mut().rust_mut().reader.busy = false;
                controller.as_mut().reader_changed();
            });
        });
        self.as_mut().rust_mut().reader.loading.task = Some(task);
    }

    pub fn select_chapter(mut self: Pin<&mut Self>, id: QString) {
        if !self.reader.open {
            return;
        }
        let Some(chapter) = self.reader.chapters.find(&id.to_string()).cloned() else {
            return;
        };
        if Some(&chapter.reference.variant) != self.reader.reference.as_ref() {
            return;
        }
        if self.reader.chapter_id == id && self.reader.error.is_empty() {
            return;
        }
        self.as_mut().rust_mut().reader.chapter.cancel();
        let generation = self.reader.chapter.generation;
        let (sender, position) = watch::channel(0usize);
        {
            let mut state = self.as_mut().rust_mut();
            state.reader.chapter_id = id;
            state.reader.chapter_title = ffi::chapter_title(
                &QString::from(chapter.number.as_deref().unwrap_or_default()),
                &QString::from(chapter.volume.as_deref().unwrap_or_default()),
                &QString::from(chapter.title.as_deref().unwrap_or_default()),
            );
            state.reader.page_index = 0;
            state.reader.page_count = 0;
            state.reader.page_url = QString::default();
            state.reader.error = QString::default();
            state.reader.busy = true;
            state.reader.position = Some(sender);
        }
        self.as_mut().reader_changed();
        if !self.reader.open || self.reader.chapter.generation != generation {
            return;
        }
        let qt_thread = self.qt_thread();
        let services = self.services.as_ref().unwrap();
        let manga = Arc::clone(&services.manga);
        let task = services.runtime.spawn(read_chapter(
            manga,
            chapter.reference,
            position,
            qt_thread,
            generation,
        ));
        self.as_mut().rust_mut().reader.chapter.task = Some(task);
    }

    pub fn select_reader_page(mut self: Pin<&mut Self>, index: i32) {
        if !self.reader.open || index < 0 || index >= self.reader.page_count {
            return;
        }
        if index == self.reader.page_index {
            return;
        }
        {
            let mut state = self.as_mut().rust_mut();
            state.reader.page_index = index;
            state.reader.page_url = QString::default();
            state.reader.error = QString::default();
            state.reader.busy = true;
            if let Some(position) = &state.reader.position {
                position.send_replace(index as usize);
            }
        }
        self.as_mut().reader_changed();
    }

    pub fn turn_reader_page(self: Pin<&mut Self>, offset: i32) {
        let index = self.reader.page_index.saturating_add(offset);
        self.select_reader_page(index);
    }

    pub fn set_reader_fit(mut self: Pin<&mut Self>, fit: ffi::ReaderFit) {
        if fit != ffi::ReaderFit::Height && fit != ffi::ReaderFit::Width {
            return;
        }
        if self.reader.fit != fit {
            self.as_mut().rust_mut().reader.fit = fit;
            self.as_mut().reader_changed();
        }
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

    pub fn retry_reader(mut self: Pin<&mut Self>) {
        if !self.reader.open || self.reader.error.is_empty() {
            return;
        }
        if self.reader.chapter_id.is_empty() {
            self.load_chapters();
        } else if self.reader.page_count == 0 {
            let chapter = self.reader.chapter_id.clone();
            self.select_chapter(chapter);
        } else {
            let mut state = self.as_mut().rust_mut();
            state.reader.error = QString::default();
            state.reader.busy = true;
            if let Some(position) = &state.reader.position {
                position.send_replace(state.reader.page_index as usize);
            }
            drop(state);
            self.as_mut().reader_changed();
        }
    }
}

struct CachedPage {
    _file: TempPath,
    url: String,
}

async fn read_chapter(
    manga: Arc<MangaDex>,
    reference: ChapterRef,
    mut position: watch::Receiver<usize>,
    qt_thread: CxxQtThread<ffi::ContentController>,
    generation: u64,
) {
    let pages = match manga.chapter_pages(&reference).await {
        Ok(pages) if !pages.images.is_empty() && pages.reference == reference => pages,
        Ok(_) => {
            publish_error(
                &qt_thread,
                generation,
                None,
                3,
                "The chapter manifest is empty or mismatched",
            );
            return;
        }
        Err(error) => {
            publish_error(&qt_thread, generation, None, 1, &error.to_string());
            return;
        }
    };
    let count = pages.images.len() as i32;
    let _ = qt_thread.queue(move |mut controller| {
        if controller.reader.chapter.generation == generation && controller.reader.open {
            controller.as_mut().rust_mut().reader.page_count = count;
            controller.as_mut().reader_changed();
        }
    });
    let directory = match tokio::task::spawn_blocking(|| {
        tempfile::Builder::new().prefix("qbutt-reader-").tempdir()
    })
    .await
    {
        Ok(Ok(directory)) => Arc::new(directory),
        result => {
            publish_error(
                &qt_thread,
                generation,
                Some(0),
                2,
                &format!("Could not create reader cache: {result:?}"),
            );
            return;
        }
    };
    let mut cache = BTreeMap::<usize, CachedPage>::new();

    // The async task alone owns cached files. The GUI holds URLs and the watch
    // sender, so cancellation and cache pruning never delete files on the GUI.
    'position: loop {
        let target = *position.borrow_and_update();
        cache.retain(|index, _| *index >= target && *index <= target.saturating_add(2));
        for index in target..(target.saturating_add(3)).min(pages.images.len()) {
            if !cache.contains_key(&index) {
                let load = load_page(&manga, &pages, index, Arc::clone(&directory));
                let result = tokio::select! {
                    result = load => result,
                    changed = position.changed() => {
                        if changed.is_err() { break 'position; }
                        continue 'position;
                    }
                };
                match result {
                    Ok(page) => {
                        cache.insert(index, page);
                    }
                    Err(error) => {
                        if index == target {
                            publish_error(&qt_thread, generation, Some(index), 2, &error);
                        } else {
                            ffi::log_content_error(
                                &QString::from("reader preload"),
                                &QString::from(error),
                            );
                        }
                        break;
                    }
                }
            }
            if index == target {
                let url = cache[&index].url.clone();
                let _ = qt_thread.queue(move |mut controller| {
                    if controller.reader.chapter.generation != generation
                        || !controller.reader.open
                        || controller.reader.page_index != index as i32
                    {
                        return;
                    }
                    let mut state = controller.as_mut().rust_mut();
                    state.reader.page_url = QString::from(url);
                    state.reader.busy = false;
                    state.reader.error = QString::default();
                    drop(state);
                    controller.as_mut().reader_changed();
                });
            }
        }
        if position.changed().await.is_err() {
            break;
        }
    }
}

async fn load_page(
    manga: &MangaDex,
    pages: &ChapterPages,
    index: usize,
    directory: Arc<TempDir>,
) -> Result<CachedPage, String> {
    let bytes = manga
        .image_bytes(&pages.images[index])
        .await
        .map_err(|error| error.to_string())?;
    tokio::task::spawn_blocking(move || {
        let mut file =
            NamedTempFile::new_in(directory.path()).map_err(|error| error.to_string())?;
        file.write_all(&bytes).map_err(|error| error.to_string())?;
        file.flush().map_err(|error| error.to_string())?;
        let path = QString::from(file.path().to_string_lossy().as_ref());
        let url = ffi::validated_image_url(&path).to_string();
        if url.is_empty() {
            return Err("The source returned an invalid or oversized image".into());
        }
        Ok(CachedPage {
            _file: file.into_temp_path(),
            url,
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

fn publish_error(
    qt_thread: &CxxQtThread<ffi::ContentController>,
    generation: u64,
    page: Option<usize>,
    operation: i32,
    detail: &str,
) {
    ffi::log_content_error(&QString::from("reader"), &QString::from(detail));
    let _ = qt_thread.queue(move |mut controller| {
        if controller.reader.chapter.generation != generation
            || !controller.reader.open
            || page.is_some_and(|index| controller.reader.page_index != index as i32)
        {
            return;
        }
        let mut state = controller.as_mut().rust_mut();
        state.reader.error = ffi::reader_error(operation);
        state.reader.busy = false;
        drop(state);
        controller.as_mut().reader_changed();
    });
}
