//! Translation data shares the profile library's one SQLite owner. Originals
//! and reader progress are never replaced by these reversible cached layers.

use rusqlite::{OptionalExtension, TransactionBehavior, params, types::ValueRef};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::{Library, LibraryError, LibraryResult};
use crate::translation::context::{ContextAdditions, EvidenceSegment, TranslationContext};
use crate::{ChapterRef, PublicationRef};

const MAX_CACHED_BYTES: usize = 4 * 1024 * 1024;
const MAX_CACHED_IMAGE_BYTES: usize = 16 * 1024 * 1024;
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// Exact identity of an accepted overlay, independent of later work context.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranslationCacheKey {
    pub chapter: ChapterRef,
    pub page: Option<u32>,
    pub target_language: String,
    pub original_digest: String,
    pub provider: String,
    pub model: String,
    pub effort: String,
    pub pipeline: u32,
}

/// The chosen Codex identity for one target language in this profile.
/// `explicit` keeps a future user-selected model from being replaced by the
/// discovered default when a chapter has no cached translation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranslationSelection {
    pub target_language: String,
    pub model: String,
    pub effort: String,
    pub explicit: bool,
}

impl TranslationSelection {
    pub fn validate(&self) -> LibraryResult<()> {
        if !valid_selection_fields(&self.target_language, &self.model, &self.effort) {
            return Err(LibraryError::InvalidEntry(
                "translation selection is invalid",
            ));
        }
        Ok(())
    }
}

impl TranslationCacheKey {
    pub fn validate(&self) -> LibraryResult<()> {
        self.chapter
            .validate()
            .map_err(|_| LibraryError::InvalidEntry("translation chapter is invalid"))?;
        if !valid_selection_fields(&self.target_language, &self.model, &self.effort)
            || self.original_digest.len() != 64
            || !self
                .original_digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || self.provider.is_empty()
            || self.provider.len() > 32
            || self.provider.chars().any(char::is_control)
            || self.pipeline == 0
        {
            return Err(LibraryError::InvalidEntry(
                "translation cache key is invalid",
            ));
        }
        Ok(())
    }
}

impl Library {
    pub fn translation_selection(
        &self,
        target_language: &str,
    ) -> LibraryResult<Option<TranslationSelection>> {
        validate_language(target_language)?;
        read_selection(&self.connection, target_language)
    }

    /// Record a discovered default after an exact cache hit, or save an
    /// explicit user choice. Accepted translations record their selection in
    /// the same transaction as their payload and context.
    pub fn save_translation_selection(
        &mut self,
        selection: &TranslationSelection,
    ) -> LibraryResult<()> {
        selection.validate()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        write_selection(&transaction, selection)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn reset_translation_selection(&mut self, target_language: &str) -> LibraryResult<()> {
        validate_language(target_language)?;
        self.connection.execute(
            "UPDATE translation_selection SET explicit = 0 WHERE target_language = ?1",
            [target_language],
        )?;
        Ok(())
    }

    pub fn translation_context(
        &self,
        publication: &PublicationRef,
        target_language: &str,
    ) -> LibraryResult<TranslationContext> {
        publication
            .validate()
            .map_err(|_| LibraryError::InvalidEntry("translation publication is invalid"))?;
        validate_language(target_language)?;
        read_context(&self.connection, publication, target_language)
    }

    pub fn cached_translation<T: DeserializeOwned>(
        &self,
        key: &TranslationCacheKey,
    ) -> LibraryResult<Option<T>> {
        key.validate()?;
        let key = serde_json::to_string(key)?;
        let content: Option<String> = self
            .connection
            .query_row(
                "SELECT content FROM translated_content WHERE key = ?1",
                [&key],
                |row| row.get(0),
            )
            .optional()?;
        content
            .map(|content| serde_json::from_str(&content).map_err(LibraryError::from))
            .transpose()
    }

    pub fn cached_translation_image(
        &self,
        key: &TranslationCacheKey,
    ) -> LibraryResult<Option<Vec<u8>>> {
        key.validate()?;
        let mut statement = self
            .connection
            .prepare("SELECT png FROM translated_images WHERE key = ?1")?;
        let mut rows = statement.query([serde_json::to_string(key)?])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let ValueRef::Blob(png) = row.get_ref(0)? else {
            return Err(LibraryError::CorruptEntry(
                "translation image is not a blob",
            ));
        };
        if png.len() > MAX_CACHED_IMAGE_BYTES || !png.starts_with(PNG_SIGNATURE) {
            return Err(LibraryError::CorruptEntry(
                "translation image is not a bounded PNG",
            ));
        }
        Ok(Some(png.to_vec()))
    }

    /// Atomically accept verified context and the translated overlay. A job
    /// begun on a stale revision must rerun with the newly confirmed context.
    pub fn save_translation<T: Serialize>(
        &mut self,
        key: TranslationCacheKey,
        expected_context_revision: u64,
        content: &T,
        additions: ContextAdditions,
        evidence: &[EvidenceSegment],
    ) -> LibraryResult<TranslationContext> {
        key.validate()?;
        let encoded_content = serde_json::to_string(content)?;
        if encoded_content.len() > MAX_CACHED_BYTES {
            return Err(LibraryError::InvalidEntry(
                "translated content exceeds its limit",
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let context = accept_translation_context(
            &transaction,
            &key,
            expected_context_revision,
            additions,
            evidence,
        )?;
        transaction.execute(
            "INSERT INTO translated_content (key, content) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET content = excluded.content",
            params![serde_json::to_string(&key)?, encoded_content],
        )?;
        transaction.commit()?;
        Ok(context)
    }

    /// Accept PNG bytes and their context together. The Qt consumer owns
    /// decoding and geometry validation before accepting a generated image.
    pub fn save_image_translation(
        &mut self,
        key: TranslationCacheKey,
        expected_context_revision: u64,
        png: &[u8],
        additions: ContextAdditions,
        evidence: &[EvidenceSegment],
    ) -> LibraryResult<TranslationContext> {
        key.validate()?;
        if png.len() > MAX_CACHED_IMAGE_BYTES || !png.starts_with(PNG_SIGNATURE) {
            return Err(LibraryError::InvalidEntry(
                "translation image is not a bounded PNG",
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let context = accept_translation_context(
            &transaction,
            &key,
            expected_context_revision,
            additions,
            evidence,
        )?;
        transaction.execute(
            "INSERT INTO translated_images (key, png) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET png = excluded.png",
            params![serde_json::to_string(&key)?, png],
        )?;
        transaction.commit()?;
        Ok(context)
    }
}

fn accept_translation_context(
    transaction: &rusqlite::Transaction<'_>,
    key: &TranslationCacheKey,
    expected_revision: u64,
    additions: ContextAdditions,
    evidence: &[EvidenceSegment],
) -> LibraryResult<TranslationContext> {
    if evidence.iter().any(|segment| {
        segment.image_observation.as_ref().is_some_and(|image| {
            segment.chapter != key.chapter
                || Some(image.page) != key.page
                || image.original_digest != key.original_digest
        })
    }) {
        return Err(LibraryError::InvalidEntry(
            "image evidence does not match the original page",
        ));
    }
    let mut context = read_context(
        transaction,
        &key.chapter.variant.publication,
        &key.target_language,
    )?;
    if context.revision != expected_revision {
        return Err(LibraryError::InvalidEntry(
            "translation context changed during the job",
        ));
    }
    if context
        .apply(&key.chapter.variant.publication, additions, evidence)
        .map_err(LibraryError::InvalidEntry)?
    {
        transaction.execute(
            "INSERT INTO translation_context (publication, language, revision, context)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (publication, language) DO UPDATE SET
                 revision = excluded.revision,
                 context = excluded.context",
            params![
                serde_json::to_string(&key.chapter.variant.publication)?,
                key.target_language,
                i64::try_from(context.revision).map_err(|_| {
                    LibraryError::InvalidEntry("translation context revision overflow")
                })?,
                serde_json::to_string(&context)?,
            ],
        )?;
    }
    write_selection(
        transaction,
        &TranslationSelection {
            target_language: key.target_language.clone(),
            model: key.model.clone(),
            effort: key.effort.clone(),
            explicit: false,
        },
    )?;
    Ok(context)
}

fn read_selection(
    connection: &rusqlite::Connection,
    target_language: &str,
) -> LibraryResult<Option<TranslationSelection>> {
    let stored: Option<(String, String, i64)> = connection
        .query_row(
            "SELECT model, effort, explicit FROM translation_selection WHERE target_language = ?1",
            [target_language],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((model, effort, explicit)) = stored else {
        return Ok(None);
    };
    let explicit = match explicit {
        0 => false,
        1 => true,
        _ => {
            return Err(LibraryError::CorruptEntry(
                "translation selection is invalid",
            ));
        }
    };
    let selection = TranslationSelection {
        target_language: target_language.to_owned(),
        model,
        effort,
        explicit,
    };
    selection
        .validate()
        .map_err(|_| LibraryError::CorruptEntry("translation selection is invalid"))?;
    Ok(Some(selection))
}

fn write_selection(
    connection: &rusqlite::Connection,
    selection: &TranslationSelection,
) -> LibraryResult<()> {
    selection.validate()?;
    if let Some(saved) = read_selection(connection, &selection.target_language)? {
        if saved.explicit && !selection.explicit {
            if saved.model != selection.model || saved.effort != selection.effort {
                return Err(LibraryError::InvalidEntry(
                    "explicit translation selection changed during the job",
                ));
            }
            return Ok(());
        }
    }
    connection.execute(
        "INSERT INTO translation_selection (target_language, model, effort, explicit)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (target_language) DO UPDATE SET
             model = excluded.model,
             effort = excluded.effort,
             explicit = excluded.explicit",
        params![
            selection.target_language,
            selection.model,
            selection.effort,
            if selection.explicit { 1 } else { 0 },
        ],
    )?;
    Ok(())
}

fn read_context(
    connection: &rusqlite::Connection,
    publication: &PublicationRef,
    language: &str,
) -> LibraryResult<TranslationContext> {
    let stored: Option<(i64, String)> = connection
        .query_row(
            "SELECT revision, context FROM translation_context
             WHERE publication = ?1 AND language = ?2",
            params![serde_json::to_string(publication)?, language],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((revision, encoded)) = stored else {
        return Ok(TranslationContext::default());
    };
    let context: TranslationContext = serde_json::from_str(&encoded)?;
    if revision < 0 || context.revision != revision as u64 || context.validate(publication).is_err()
    {
        return Err(LibraryError::CorruptEntry("translation context is invalid"));
    }
    Ok(context)
}

fn validate_language(language: &str) -> LibraryResult<()> {
    if !crate::model::canonical_language(language) {
        return Err(LibraryError::InvalidEntry(
            "translation language is invalid",
        ));
    }
    Ok(())
}

fn valid_selection_fields(language: &str, model: &str, effort: &str) -> bool {
    crate::model::canonical_language(language)
        && !model.is_empty()
        && model.len() <= 128
        && !model.chars().any(char::is_control)
        && !effort.is_empty()
        && effort.len() <= 32
        && !effort.chars().any(char::is_control)
}
