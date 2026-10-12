use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use qbutt_content::{Source, Transport, VariantFilters};

use super::ffi;
use super::library::LibraryAction;

impl ffi::ContentController {
    fn selected_filters(&self) -> Option<&VariantFilters> {
        let work = self.selected_work.as_ref()?;
        self.work_preferences
            .get(&work.reference)
            .map(|preferences| &preferences.filters)
    }

    pub(super) fn variant_episode_filter(&self) -> QString {
        QString::from(
            self.selected_filters()
                .and_then(|filters| filters.episode.as_deref())
                .unwrap_or_default(),
        )
    }

    pub(super) fn variant_translation_filter(&self) -> QString {
        QString::from(
            self.selected_filters()
                .and_then(|filters| filters.translation.as_deref())
                .unwrap_or_default(),
        )
    }

    pub(super) fn variant_source_filter(&self) -> QString {
        QString::from(
            self.selected_filters()
                .and_then(|filters| filters.source)
                .map(Source::id)
                .unwrap_or_default(),
        )
    }

    pub(super) fn variant_player_filter(&self) -> QString {
        QString::from(
            self.selected_filters()
                .and_then(|filters| filters.player.as_deref())
                .unwrap_or_default(),
        )
    }

    pub(super) fn variant_language_filter(&self) -> QString {
        QString::from(
            self.selected_filters()
                .and_then(|filters| filters.language.as_deref())
                .unwrap_or_default(),
        )
    }

    pub(super) fn variant_transport_filter(&self) -> QString {
        QString::from(
            match self
                .selected_filters()
                .and_then(|filters| filters.transport)
            {
                Some(Transport::Torrent) => "torrent",
                Some(Transport::Video) => "video",
                Some(Transport::Manga) => "manga",
                None => "",
            },
        )
    }

    pub(super) fn variant_query(&self) -> QString {
        QString::from(
            self.selected_filters()
                .and_then(|filters| filters.query.as_deref())
                .unwrap_or_default(),
        )
    }

    pub fn set_variant_episode_filter(self: Pin<&mut Self>, value: QString) {
        self.update_variant_filters(|filters| filters.episode = optional_text(value));
    }

    pub fn set_variant_translation_filter(self: Pin<&mut Self>, value: QString) {
        self.update_variant_filters(|filters| filters.translation = optional_text(value));
    }

    pub fn set_variant_provider_filter(self: Pin<&mut Self>, source: QString, player: QString) {
        let source_id = source.to_string();
        let source = Source::from_id(&source_id);
        if !source_id.is_empty() && source.is_none() {
            return;
        }
        self.update_variant_filters(|filters| {
            filters.source = source;
            filters.player = optional_text(player);
        });
    }

    pub fn set_variant_language_filter(self: Pin<&mut Self>, value: QString) {
        self.update_variant_filters(|filters| filters.language = optional_text(value));
    }

    pub fn set_variant_query(self: Pin<&mut Self>, value: QString) {
        self.update_variant_filters(|filters| filters.query = optional_text(value));
    }

    pub fn clear_variant_filters(self: Pin<&mut Self>) {
        self.update_variant_filters(|filters| *filters = VariantFilters::default());
    }

    fn update_variant_filters(mut self: Pin<&mut Self>, update: impl FnOnce(&mut VariantFilters)) {
        if self.library_state != ffi::LibraryState::Ready {
            return;
        }
        let Some(work) = self.selected_work.clone() else {
            return;
        };
        let previous = self.work_preferences.get(&work.reference).cloned();
        let mut preferences = previous.clone().unwrap_or_default();
        update(&mut preferences.filters);
        if previous.as_ref() == Some(&preferences)
            && self.library_error_kind != ffi::LibraryErrorKind::Writing
        {
            return;
        }
        let reference = work.reference.clone();
        self.as_mut()
            .rust_mut()
            .work_preferences
            .insert(reference.clone(), preferences.clone());
        let accepted = self.as_mut().submit_library(LibraryAction::Preferences {
            work,
            preferences: preferences.clone(),
        });
        if !accepted && self.work_preferences.get(&reference) == Some(&preferences) {
            let mut state = self.as_mut().rust_mut();
            if let Some(previous) = previous {
                state.work_preferences.insert(reference, previous);
            } else {
                state.work_preferences.remove(&reference);
            }
        }
        self.as_mut().variant_filters_changed();
    }
}

fn optional_text(value: QString) -> Option<String> {
    let value = value.to_string();
    (!value.is_empty()).then_some(value)
}
