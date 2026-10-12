use std::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use qbutt_content::TranslationSelection;
use qbutt_content::translation::codex::{self, CodexModel};

use super::ffi;

#[derive(Default)]
pub(super) struct TranslationOptionsState {
    models: Vec<CodexModel>,
    selection: Option<TranslationSelection>,
    language: String,
    generation: u64,
    loading: bool,
    error: bool,
}

impl ffi::ContentController {
    pub(super) fn reader_translation_model_count(&self) -> i32 {
        self.translation_options.models.len() as i32
    }

    pub fn reader_translation_model_at(&self, index: i32) -> QString {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.translation_options.models.get(index))
            .map(|model| QString::from(&model.id))
            .unwrap_or_default()
    }

    pub(super) fn reader_translation_effort_count(&self) -> i32 {
        let selected = self.reader_translation_model().to_string();
        self.translation_options
            .models
            .iter()
            .find(|model| model.id == selected)
            .map_or(0, |model| model.efforts.len() as i32)
    }

    pub fn reader_translation_effort_at(&self, index: i32) -> QString {
        let selected = self.reader_translation_model().to_string();
        usize::try_from(index)
            .ok()
            .and_then(|index| {
                self.translation_options
                    .models
                    .iter()
                    .find(|model| model.id == selected)?
                    .efforts
                    .get(index)
            })
            .map(QString::from)
            .unwrap_or_default()
    }

    pub(super) fn reader_translation_model(&self) -> QString {
        self.translation_options
            .selection
            .as_ref()
            .filter(|selection| selection.explicit)
            .map(|selection| QString::from(&selection.model))
            .unwrap_or_default()
    }

    pub(super) fn reader_translation_effort(&self) -> QString {
        self.translation_options
            .selection
            .as_ref()
            .filter(|selection| selection.explicit)
            .map(|selection| QString::from(&selection.effort))
            .unwrap_or_default()
    }

    pub(super) fn reader_translation_options_loading(&self) -> bool {
        self.translation_options.loading
    }

    pub(super) fn reader_translation_options_error(&self) -> bool {
        self.translation_options.error
    }

    pub fn load_reader_translation_options(mut self: Pin<&mut Self>) -> bool {
        let (Some(library), Some(runtime)) = (
            self.library_worker.as_ref().cloned(),
            self.services
                .as_ref()
                .map(|services| services.runtime.handle().clone()),
        ) else {
            return false;
        };
        let language = self.reader_settings.value.translation_language.clone();
        let qt_thread = self.qt_thread();
        let generation = self.translation_options.generation.wrapping_add(1);
        {
            let state = &mut self.as_mut().rust_mut().translation_options;
            state.generation = generation;
            state.language = language.clone();
            state.loading = true;
            state.error = false;
            state.models.clear();
            state.selection = None;
        }
        self.as_mut().reader_translation_options_changed();
        runtime.spawn(async move {
            let (selection, models) = tokio::join!(
                library.translation_selection(language.clone()),
                codex::available_models()
            );
            let _ = qt_thread.queue(move |mut controller| {
                if controller.translation_options.generation != generation
                    || controller.translation_options.language != language
                {
                    return;
                }
                let state = &mut controller.as_mut().rust_mut().translation_options;
                state.loading = false;
                match selection {
                    Ok(selection) => state.selection = selection,
                    Err(error) => {
                        state.error = true;
                        ffi::log_content_error(
                            &QString::from("translation selection"),
                            &QString::from(error.to_string()),
                        );
                    }
                }
                match models {
                    Ok(models) => state.models = models,
                    Err(error) => {
                        state.error = true;
                        ffi::log_content_error(
                            &QString::from("Codex model catalog"),
                            &QString::from(error.to_string()),
                        );
                    }
                }
                controller.as_mut().reader_translation_options_changed();
            });
        });
        true
    }

    pub fn set_reader_translation_model(mut self: Pin<&mut Self>, model: QString) -> bool {
        if self.translation_options.loading {
            return false;
        }
        let model = model.to_string();
        let choice = if model.is_empty() {
            self.translation_options
                .selection
                .clone()
                .map(|mut selection| {
                    selection.explicit = false;
                    selection
                })
        } else {
            let Some(available) = self
                .translation_options
                .models
                .iter()
                .find(|available| available.id == model)
            else {
                return false;
            };
            Some(TranslationSelection {
                target_language: self.reader_settings.value.translation_language.clone(),
                model,
                effort: available.default_effort.clone(),
                explicit: true,
            })
        };
        self.as_mut().save_reader_translation_choice(choice)
    }

    pub fn set_reader_translation_effort(mut self: Pin<&mut Self>, effort: QString) -> bool {
        if self.translation_options.loading {
            return false;
        }
        let Some(mut choice) = self.translation_options.selection.clone() else {
            return false;
        };
        if !choice.explicit
            || !self.translation_options.models.iter().any(|model| {
                model.id == choice.model
                    && model
                        .efforts
                        .iter()
                        .any(|supported| supported == &effort.to_string())
            })
        {
            return false;
        }
        choice.effort = effort.to_string();
        self.as_mut().save_reader_translation_choice(Some(choice))
    }

    fn save_reader_translation_choice(
        mut self: Pin<&mut Self>,
        choice: Option<TranslationSelection>,
    ) -> bool {
        let Some(library) = self.library_worker.as_ref().cloned() else {
            return false;
        };
        let Some(runtime) = self
            .services
            .as_ref()
            .map(|services| services.runtime.handle().clone())
        else {
            return false;
        };
        let language = self.reader_settings.value.translation_language.clone();
        if self.translation_options.language != language {
            return false;
        }
        if self
            .translation_options
            .selection
            .as_ref()
            .filter(|saved| saved.explicit)
            == choice.as_ref().filter(|selected| selected.explicit)
        {
            return true;
        }
        let was_translated = self.reader_translation_requested();
        if was_translated {
            self.as_mut().reset_reader_translation();
        }
        let qt_thread = self.qt_thread();
        let generation = self.translation_options.generation.wrapping_add(1);
        {
            let state = &mut self.as_mut().rust_mut().translation_options;
            state.generation = generation;
            state.loading = true;
            state.error = false;
        }
        self.as_mut().reader_translation_options_changed();
        runtime.spawn(async move {
            let result =
                if let Some(selection) = choice.as_ref().filter(|selection| selection.explicit) {
                    library
                        .remember_translation_selection(selection.clone())
                        .await
                } else {
                    library.reset_translation_selection(language.clone()).await
                };
            let _ = qt_thread.queue(move |mut controller| {
                if controller.translation_options.generation != generation
                    || controller.translation_options.language != language
                {
                    return;
                }
                let state = &mut controller.as_mut().rust_mut().translation_options;
                state.loading = false;
                if let Err(error) = result {
                    state.error = true;
                    ffi::log_content_error(
                        &QString::from("translation selection"),
                        &QString::from(error.to_string()),
                    );
                } else {
                    state.selection = choice;
                }
                controller.as_mut().reader_translation_options_changed();
                if was_translated {
                    controller.as_mut().restore_reader_translation();
                }
            });
        });
        true
    }
}
