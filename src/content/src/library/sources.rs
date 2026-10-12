use rusqlite::{Connection, TransactionBehavior, params};

use crate::{Source, SourceFailure, SourceFailureReason, SourceOperation, SourceSettings};

use super::{Library, LibraryError, LibraryResult};

impl Library {
    pub fn source_settings(&self) -> LibraryResult<Vec<SourceSettings>> {
        let mut statement = self.connection.prepare(
            "SELECT source, enabled, revision, failure FROM source_settings ORDER BY source",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?;
        let mut settings = Vec::with_capacity(Source::ALL.len());
        for row in rows {
            let (id, enabled, revision, failure) = row?;
            let source = Source::from_id(&id).ok_or(LibraryError::InvalidSchema)?;
            settings.push(decode(source, enabled, revision, failure)?);
        }
        if settings.len() != Source::ALL.len() {
            return Err(LibraryError::InvalidSchema);
        }
        Ok(settings)
    }

    pub fn set_source_enabled(
        &mut self,
        source: Source,
        enabled: bool,
    ) -> LibraryResult<SourceSettings> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "UPDATE source_settings SET enabled = ?1, revision = revision + 1
             WHERE source = ?2 AND enabled != ?1",
            params![i64::from(enabled), source.id()],
        )?;
        let settings = read(&transaction, source)?;
        transaction.commit()?;
        Ok(settings)
    }

    pub fn record_source_failure(
        &mut self,
        failure: SourceFailure,
    ) -> LibraryResult<SourceSettings> {
        validate_failure(&failure)?;
        let source = failure.operation.source();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if transaction.execute(
            "UPDATE source_settings SET failure = ?1, revision = revision + 1 WHERE source = ?2",
            params![serde_json::to_string(&failure)?, source.id()],
        )? != 1
        {
            return Err(LibraryError::InvalidSchema);
        }
        let settings = read(&transaction, source)?;
        transaction.commit()?;
        Ok(settings)
    }

    /// Success cannot clear another operation, a newer failure or a changed
    /// configuration. The caller captures this revision before acquisition.
    pub fn clear_source_failure(
        &mut self,
        operation: &SourceOperation,
        observed_revision: i64,
    ) -> LibraryResult<SourceSettings> {
        operation
            .validate()
            .map_err(|_| LibraryError::InvalidEntry("source operation"))?;
        if observed_revision < 0 {
            return Err(LibraryError::InvalidEntry("source revision"));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let source = operation.source();
        let mut settings = read(&transaction, source)?;
        if settings.revision == observed_revision
            && settings.failure.as_ref().map(|failure| &failure.operation) == Some(operation)
        {
            transaction.execute(
                "UPDATE source_settings SET failure = NULL, revision = revision + 1 WHERE source = ?1",
                [source.id()],
            )?;
            settings = read(&transaction, source)?;
        }
        transaction.commit()?;
        Ok(settings)
    }
}

fn read(connection: &Connection, source: Source) -> LibraryResult<SourceSettings> {
    let (enabled, revision, failure) = connection.query_row(
        "SELECT enabled, revision, failure FROM source_settings WHERE source = ?1",
        [source.id()],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        },
    )?;
    decode(source, enabled, revision, failure)
}

fn decode(
    source: Source,
    enabled: i64,
    revision: i64,
    failure: Option<String>,
) -> LibraryResult<SourceSettings> {
    if !matches!(enabled, 0 | 1) || revision < 0 {
        return Err(LibraryError::CorruptEntry("source settings"));
    }
    let failure = failure
        .as_deref()
        .map(serde_json::from_str::<SourceFailure>)
        .transpose()?;
    if let Some(failure) = &failure {
        validate_failure(failure).map_err(|_| LibraryError::CorruptEntry("source failure"))?;
        if failure.operation.source() != source {
            return Err(LibraryError::CorruptEntry("source failure identity"));
        }
    }
    Ok(SourceSettings {
        source,
        enabled: enabled != 0,
        revision,
        failure,
    })
}

fn validate_failure(failure: &SourceFailure) -> LibraryResult<()> {
    failure
        .operation
        .validate()
        .map_err(|_| LibraryError::InvalidEntry("source operation"))?;
    if matches!(failure.reason, SourceFailureReason::Http(status) if !(100..=599).contains(&status))
    {
        return Err(LibraryError::InvalidEntry("source HTTP status"));
    }
    Ok(())
}
