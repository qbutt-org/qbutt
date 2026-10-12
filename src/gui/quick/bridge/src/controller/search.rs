use std::pin::Pin;
use std::sync::Arc;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use qbutt_content::{Category, Source};
use tokio::task::JoinSet;

use super::ffi;

const MAX_SEARCH_REQUESTS: usize = 4;

impl ffi::ContentController {
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
        let anime = Arc::clone(&services.anime);
        let manga = Arc::clone(&services.manga);
        let rutor = Arc::clone(&services.rutor);
        let language = services.language.clone();
        let task = services.runtime.spawn(async move {
            let mut requests = [
                (Source::AniLiberty, Category::Anime),
                (Source::MangaDex, Category::Manga),
                (Source::RuTor, Category::Movies),
                (Source::RuTor, Category::Tv),
                (Source::RuTor, Category::Anime),
                (Source::RuTor, Category::Games),
                (Source::RuTor, Category::Software),
                (Source::RuTor, Category::Books),
            ]
            .into_iter();
            let mut pending = JoinSet::new();
            let mut any_succeeded = false;
            loop {
                while pending.len() < MAX_SEARCH_REQUESTS
                    && let Some((source, category)) = requests.next()
                {
                    let anime = Arc::clone(&anime);
                    let manga = Arc::clone(&manga);
                    let rutor = Arc::clone(&rutor);
                    let query = query.clone();
                    let language = language.clone();
                    pending.spawn(async move {
                        let result = match source {
                            Source::AniLiberty => anime.search(&query).await,
                            Source::MangaDex => manga.search(&query, &language).await,
                            Source::RuTor => rutor.search(&query, category).await,
                        };
                        (source, category, result)
                    });
                }
                let Some(completed) = pending.join_next().await else {
                    break;
                };
                let (source, category, result) = match completed {
                    Ok(completed) => completed,
                    Err(error) => {
                        let _ = qt_thread.queue(move |controller| {
                            if controller.search.generation == generation {
                                ffi::log_content_error(
                                    &QString::from("search"),
                                    &QString::from(error.to_string()),
                                );
                            }
                        });
                        continue;
                    }
                };
                any_succeeded |= result.is_ok();
                let _ = qt_thread.queue(move |mut controller| {
                    if controller.search.generation != generation {
                        return;
                    }
                    match result {
                        Ok(works) => {
                            for work in works {
                                if controller.search.generation != generation {
                                    return;
                                }
                                controller.as_mut().rust_mut().works.pin_mut().update(work);
                            }
                        }
                        Err(qbutt_content::Error::InvalidQuery) => {}
                        Err(error) => ffi::log_content_error(
                            &QString::from(format!("search {source:?} {category:?}")),
                            &QString::from(error.to_string()),
                        ),
                    }
                });
            }
            let _ = qt_thread.queue(move |mut controller| {
                if controller.search.generation != generation {
                    return;
                }
                controller.as_mut().rust_mut().search.task = None;
                let state = if any_succeeded {
                    ffi::SearchState::Succeeded
                } else {
                    controller.as_mut().set_error_text(ffi::content_error(1));
                    ffi::SearchState::Failed
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
}
