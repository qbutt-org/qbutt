//! Translation data shares the profile library's one SQLite owner. Originals
//! and reader progress are never replaced by these reversible cached layers.

use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::{Library, LibraryError, LibraryResult};
use crate::translation::context::{ContextAdditions, EvidenceSegment, TranslationContext};
use crate::{ChapterRef, PublicationRef};

const MAX_CACHED_BYTES: usize = 4 * 1024 * 1024;

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
    pub context_revision: u64,
    pub pipeline: u32,
}

impl TranslationCacheKey {
    pub fn validate(&self) -> LibraryResult<()> {
        self.chapter
            .validate()
            .map_err(|_| LibraryError::InvalidEntry("translation chapter is invalid"))?;
        if !crate::model::canonical_language(&self.target_language)
            || self.original_digest.len() != 64
            || !self
                .original_digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || self.provider.is_empty()
            || self.provider.len() > 32
            || self.provider.chars().any(char::is_control)
            || self.model.is_empty()
            || self.model.len() > 128
            || self.model.chars().any(char::is_control)
            || self.effort.is_empty()
            || self.effort.len() > 32
            || self.effort.chars().any(char::is_control)
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

    /// Atomically accept verified context and the translated overlay. A job
    /// begun on a stale revision must rerun with the newly confirmed context.
    pub fn save_translation<T: Serialize>(
        &mut self,
        mut key: TranslationCacheKey,
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
        let mut context = read_context(
            &transaction,
            &key.chapter.variant.publication,
            &key.target_language,
        )?;
        if context.revision != key.context_revision {
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
        key.context_revision = context.revision;
        transaction.execute(
            "INSERT INTO translated_content (key, content) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET content = excluded.content",
            params![serde_json::to_string(&key)?, encoded_content],
        )?;
        transaction.commit()?;
        Ok(context)
    }
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
