use std::collections::{BTreeMap, BTreeSet, btree_map::Entry};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use cxx_qt::CxxQtThread;
use cxx_qt_lib::{QString, QUrl};
use qbutt_content::{ChapterPages, ChapterRef, MangaDex};
use tempfile::{NamedTempFile, TempDir, TempPath};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};

use super::ffi;
use super::reader::ReaderViewport;

pub(crate) struct CachedPage {
    file: TempPath,
    directory: Arc<ReaderDirectory>,
    pub(crate) url: String,
    pub(crate) width: i32,
    pub(crate) height: i32,
    bytes: Option<OwnedSemaphorePermit>,
}

impl Drop for CachedPage {
    fn drop(&mut self) {
        self.file.disable_cleanup(true);
        ffi::retire_reader_cache(Box::new(RetiredReaderCache {
            path: RetiredReaderPath::File {
                path: self.file.to_path_buf(),
                _directory: Arc::clone(&self.directory),
                _bytes: self.bytes.take(),
            },
        }));
    }
}

struct ReaderDirectory(TempDir);

impl Drop for ReaderDirectory {
    fn drop(&mut self) {
        self.0.disable_cleanup(true);
        ffi::retire_reader_cache(Box::new(RetiredReaderCache {
            path: RetiredReaderPath::Directory(self.0.path().to_path_buf()),
        }));
    }
}

pub(crate) struct RetiredReaderCache {
    path: RetiredReaderPath,
}

enum RetiredReaderPath {
    File {
        path: PathBuf,
        _directory: Arc<ReaderDirectory>,
        _bytes: Option<OwnedSemaphorePermit>,
    },
    Directory(PathBuf),
}

pub(super) fn clean_retired_reader_cache(cache: &RetiredReaderCache) {
    // Qt may still be decoding the retired file asynchronously. The global
    // pool owns cleanup after the controller and its Tokio runtime are gone.
    // File jobs retain the directory, so its cleanup cannot race their unlink.
    let path = match &cache.path {
        RetiredReaderPath::File { path, .. } | RetiredReaderPath::Directory(path) => path,
    };
    loop {
        let result = match &cache.path {
            RetiredReaderPath::File { .. } => std::fs::remove_file(path),
            RetiredReaderPath::Directory(_) => std::fs::remove_dir_all(path),
        };
        let Err(error) = result else {
            return;
        };
        if error.kind() == std::io::ErrorKind::NotFound {
            return;
        }
        let sharing_violation = cfg!(windows) && matches!(error.raw_os_error(), Some(32 | 33));
        // Capacity belongs to the physical file. A decoder holding it longer
        // than a timeout must not release bytes or strand a live cache payload.
        if sharing_violation {
            std::thread::sleep(Duration::from_millis(25));
            continue;
        }
        ffi::log_content_error(
            &QString::from("reader cache cleanup"),
            &QString::from(format!("{}: {error}", path.display())),
        );
        return;
    }
}

pub(super) enum PageEvent {
    Loading,
    Ready(Arc<CachedPage>),
    Failed,
}

pub(super) async fn read_chapter(
    manga: Arc<MangaDex>,
    reference: ChapterRef,
    mut position: watch::Receiver<ReaderViewport>,
    qt_thread: CxxQtThread<ffi::ContentController>,
    generation: u64,
    load_limit: Arc<Semaphore>,
    byte_limit: Arc<Semaphore>,
) {
    let pages = match manga.chapter_pages(&reference).await {
        Ok(pages) if !pages.images.is_empty() && pages.reference == reference => pages,
        Ok(_) => {
            publish_error(
                &qt_thread,
                generation,
                None,
                ffi::ReaderError::ChapterUnavailable,
                "The chapter manifest is empty or mismatched",
            );
            return;
        }
        Err(error) => {
            publish_error(
                &qt_thread,
                generation,
                None,
                ffi::ReaderError::Chapter,
                &error.to_string(),
            );
            return;
        }
    };
    let count = pages.images.len();
    if position.borrow().anchor >= count {
        publish_error(
            &qt_thread,
            generation,
            None,
            ffi::ReaderError::PageUnavailable,
            "The exact page is no longer available",
        );
        return;
    }
    let _ =
        qt_thread.queue(move |controller| controller.complete_reader_manifest(generation, count));
    let directory = match tokio::task::spawn_blocking(|| {
        tempfile::Builder::new()
            .prefix("qbutt-reader-")
            .tempdir()
            .map(|directory| Arc::new(ReaderDirectory(directory)))
    })
    .await
    .map_err(|error| error.to_string())
    .and_then(|result| result.map_err(|error| error.to_string()))
    {
        Ok(directory) => directory,
        Err(error) => {
            publish_error(
                &qt_thread,
                generation,
                Some(position.borrow().anchor),
                ffi::ReaderError::Page,
                &format!("Could not create reader cache: {error}"),
            );
            return;
        }
    };
    let mut cache = BTreeMap::<usize, Arc<CachedPage>>::new();
    let mut failures = BTreeSet::new();
    let mut retried = 0;
    'position: loop {
        let viewport = *position.borrow_and_update();
        let retained = viewport.retained(count);
        cache.retain(|index, _| *index == viewport.anchor || retained.contains(index));
        failures.retain(|index| *index == viewport.anchor || retained.contains(index));
        if let Some((revision, index)) = viewport.retry
            && revision != retried
        {
            retried = revision;
            cache.remove(&index);
            failures.remove(&index);
        }
        let indices = std::iter::once(viewport.anchor)
            .chain((viewport.first..=viewport.last).filter(|index| *index != viewport.anchor))
            .chain(viewport.last.saturating_add(1)..=viewport.last.saturating_add(2).min(count - 1))
            .chain(viewport.first.checked_sub(1));
        for index in indices {
            if failures.contains(&index) {
                continue;
            }
            if let Entry::Vacant(entry) = cache.entry(index) {
                let visible =
                    index == viewport.anchor || (viewport.first..=viewport.last).contains(&index);
                let reservation = if visible {
                    None
                } else {
                    // Prefetch never waits for capacity already owned by visible
                    // or retiring images. A real visible request may wait for unlink.
                    let Ok(permit) =
                        Arc::clone(&byte_limit).try_acquire_many_owned(24 * 1024 * 1024)
                    else {
                        continue;
                    };
                    Some(permit)
                };
                publish_page(&qt_thread, generation, viewport, index, PageEvent::Loading);
                let load = load_page(
                    &manga,
                    &pages,
                    index,
                    Arc::clone(&directory),
                    Arc::clone(&load_limit),
                    Arc::clone(&byte_limit),
                    reservation,
                );
                let result = tokio::select! {
                    result = load => result,
                    changed = position.changed() => {
                        if changed.is_err() { break 'position; }
                        continue 'position;
                    }
                };
                match result {
                    Ok(page) => {
                        entry.insert(Arc::new(page));
                    }
                    Err(error) => {
                        failures.insert(index);
                        ffi::log_content_error(
                            &QString::from("reader page"),
                            &QString::from(error),
                        );
                        publish_page(&qt_thread, generation, viewport, index, PageEvent::Failed);
                        if index == viewport.anchor {
                            break;
                        }
                        continue;
                    }
                }
            }
            publish_page(
                &qt_thread,
                generation,
                viewport,
                index,
                PageEvent::Ready(Arc::clone(&cache[&index])),
            );
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
    directory: Arc<ReaderDirectory>,
    load_limit: Arc<Semaphore>,
    byte_limit: Arc<Semaphore>,
    reservation: Option<OwnedSemaphorePermit>,
) -> Result<CachedPage, String> {
    let load_permit = load_limit
        .acquire_owned()
        .await
        .map_err(|error| error.to_string())?;
    let bytes = manga
        .image_bytes(&pages.images[index])
        .await
        .map_err(|error| error.to_string())?;
    let byte_permit = match reservation {
        Some(mut permit) => permit
            .split(bytes.len() as usize)
            .expect("SourceHttp image limit fits prefetch reservation"),
        None => byte_limit
            .acquire_many_owned(bytes.len() as u32)
            .await
            .map_err(|error| error.to_string())?,
    };
    tokio::task::spawn_blocking(move || {
        // Aborting the awaiting future cannot abort this closure. It retains
        // both permits through validation and file retirement on a stale result.
        let _load_permit = load_permit;
        let mut file =
            NamedTempFile::new_in(directory.0.path()).map_err(|error| error.to_string())?;
        file.write_all(&bytes).map_err(|error| error.to_string())?;
        file.flush().map_err(|error| error.to_string())?;
        let path = QString::from(file.path().to_string_lossy().as_ref());
        let size = ffi::validated_image_size(&path);
        if size.width() <= 0 || size.height() <= 0 {
            return Err("The source returned an invalid or oversized image".into());
        }
        Ok(CachedPage {
            file: file.into_temp_path(),
            directory,
            url: QUrl::from_local_file(&path).to_string(),
            width: size.width(),
            height: size.height(),
            bytes: Some(byte_permit),
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

fn publish_page(
    qt_thread: &CxxQtThread<ffi::ContentController>,
    generation: u64,
    viewport: ReaderViewport,
    index: usize,
    event: PageEvent,
) {
    let _ = qt_thread.queue(move |controller| {
        controller.complete_reader_page(generation, viewport, index, event)
    });
}

fn publish_error(
    qt_thread: &CxxQtThread<ffi::ContentController>,
    generation: u64,
    page: Option<usize>,
    error: ffi::ReaderError,
    detail: &str,
) {
    ffi::log_content_error(&QString::from("reader"), &QString::from(detail));
    let _ = qt_thread
        .queue(move |controller| controller.complete_reader_error(generation, page, error));
}
