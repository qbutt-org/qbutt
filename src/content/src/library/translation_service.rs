//! Explicit text inference settings. The same Library connection owns the
//! protected key, service choice and accepted translation transactions.

use rusqlite::{Connection, TransactionBehavior, params};

use super::{Library, LibraryError, LibraryResult};
use crate::secrets;
use crate::translation::openrouter::OpenRouter;

pub(super) const SERVICE_SCHEMA: &str = "CREATE TABLE text_translation_service (
    id INTEGER PRIMARY KEY NOT NULL CHECK (id = 1),
    service TEXT NOT NULL CHECK (service IN ('codex', 'openrouter')),
    model TEXT NOT NULL,
    effort TEXT NOT NULL,
    api_key_dpapi BLOB CHECK (api_key_dpapi IS NULL OR length(api_key_dpapi) BETWEEN 1 AND 65536)
) STRICT;
INSERT INTO text_translation_service (id, service, model, effort)
VALUES (1, 'codex', '', 'default');";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextTranslationService {
    #[default]
    Codex,
    OpenRouter,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenRouterSettings {
    pub model: String,
    pub effort: String,
}

impl Default for OpenRouterSettings {
    fn default() -> Self {
        Self {
            model: String::new(),
            effort: "default".into(),
        }
    }
}

impl OpenRouterSettings {
    pub fn valid(&self) -> bool {
        let Some((author, model)) = self.model.split_once('/') else {
            return false;
        };
        !author.is_empty()
            && !model.is_empty()
            && !model.contains('/')
            && self.model != "openrouter/auto"
            && self.model.len() <= 128
            && self
                .model
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"/._-:".contains(&byte))
            && matches!(
                self.effort.as_str(),
                "default" | "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
            )
    }
}

/// Safe metadata for the UI and for offline cache lookup. The key is never returned.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextTranslationSettings {
    pub service: TextTranslationService,
    pub openrouter: OpenRouterSettings,
    pub key_configured: bool,
}

impl Library {
    pub fn text_translation_settings(&self) -> LibraryResult<TextTranslationSettings> {
        read_settings(&self.connection)
    }

    /// None retains the saved key; an empty string explicitly removes it.
    pub fn configure_text_translation(
        &mut self,
        service: TextTranslationService,
        openrouter: &OpenRouterSettings,
        api_key: Option<&str>,
    ) -> LibraryResult<TextTranslationSettings> {
        if !settings_valid(service, openrouter) {
            return Err(LibraryError::InvalidEntry(
                "OpenRouter model or reasoning effort is invalid",
            ));
        }
        let protected =
            match api_key {
                Some("") | None => None,
                Some(key) => Some(secrets::protect(key).map_err(|_| {
                    LibraryError::Credentials("Windows could not protect the API key")
                })?),
            };
        let service = match service {
            TextTranslationService::Codex => "codex",
            TextTranslationService::OpenRouter => "openrouter",
        };
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "UPDATE text_translation_service SET service = ?1, model = ?2, effort = ?3,
                api_key_dpapi = CASE WHEN ?4 THEN ?5 ELSE api_key_dpapi END WHERE id = 1",
            params![
                service,
                openrouter.model,
                openrouter.effort,
                api_key.is_some(),
                protected
            ],
        )?;
        let saved = read_settings(&transaction)?;
        transaction.commit()?;
        Ok(saved)
    }

    /// Call only after an exact cache miss. Saved identity does not require unlock,
    /// an account check or a model-catalog request.
    pub fn openrouter_client(&self) -> LibraryResult<OpenRouter> {
        let settings = read_settings(&self.connection)?;
        if settings.service != TextTranslationService::OpenRouter || !settings.openrouter.valid() {
            return Err(LibraryError::InvalidEntry(
                "OpenRouter is not explicitly configured",
            ));
        }
        let protected: Option<Vec<u8>> = self.connection.query_row(
            "SELECT api_key_dpapi FROM text_translation_service WHERE id = 1",
            [],
            |row| row.get(0),
        )?;
        let protected = protected.ok_or(LibraryError::Credentials("Set an OpenRouter API key"))?;
        let secret = secrets::unprotect(&protected)
            .map_err(|_| LibraryError::Credentials("Windows could not unlock the saved API key"))?;
        OpenRouter::new(settings.openrouter, secret)
            .map_err(|_| LibraryError::InvalidEntry("OpenRouter transport could not be configured"))
    }
}

pub(super) fn read_settings(connection: &Connection) -> LibraryResult<TextTranslationSettings> {
    let settings = connection.query_row(
        "SELECT service, model, effort, api_key_dpapi IS NOT NULL FROM text_translation_service WHERE id = 1",
        [],
        |row| {
            let service: String = row.get(0)?;
            let service = match service.as_str() {
                "codex" => TextTranslationService::Codex,
                "openrouter" => TextTranslationService::OpenRouter,
                _ => return Err(rusqlite::Error::InvalidQuery),
            };
            Ok(TextTranslationSettings {
                service,
                openrouter: OpenRouterSettings { model: row.get(1)?, effort: row.get(2)? },
                key_configured: row.get(3)?,
            })
        },
    )?;
    if !settings_valid(settings.service, &settings.openrouter) {
        return Err(LibraryError::CorruptEntry(
            "text translation service settings are invalid",
        ));
    }
    Ok(settings)
}

fn settings_valid(service: TextTranslationService, openrouter: &OpenRouterSettings) -> bool {
    openrouter.valid()
        || (service == TextTranslationService::Codex
            && *openrouter == OpenRouterSettings::default())
}
