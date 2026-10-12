use rusqlite::TransactionBehavior;

use super::{Library, LibraryError, LibraryResult};

/// Profile-wide reading choices, independent of a publication and its progress.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReaderSettings {
    pub fit: ReaderFit,
    pub mode: ReaderMode,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReaderFit {
    #[default]
    Height,
    Width,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReaderMode {
    #[default]
    Paged,
    Continuous,
}

impl Library {
    pub fn reader_settings(&self) -> LibraryResult<ReaderSettings> {
        let mut statement = self
            .connection
            .prepare("SELECT id, fit, mode FROM reader_settings")?;
        let mut rows = statement.query([])?;
        let row = rows.next()?.ok_or(LibraryError::InvalidSchema)?;
        if row.get::<_, i64>(0)? != 1 {
            return Err(LibraryError::InvalidSchema);
        }
        let fit = match row.get::<_, i64>(1)? {
            0 => ReaderFit::Height,
            1 => ReaderFit::Width,
            _ => return Err(LibraryError::InvalidSchema),
        };
        let mode = match row.get::<_, i64>(2)? {
            0 => ReaderMode::Paged,
            1 => ReaderMode::Continuous,
            _ => return Err(LibraryError::InvalidSchema),
        };
        if rows.next()?.is_some() {
            return Err(LibraryError::InvalidSchema);
        }
        Ok(ReaderSettings { fit, mode })
    }

    pub fn save_reader_settings(&mut self, settings: ReaderSettings) -> LibraryResult<()> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let fit = match settings.fit {
            ReaderFit::Height => 0,
            ReaderFit::Width => 1,
        };
        let mode = match settings.mode {
            ReaderMode::Paged => 0,
            ReaderMode::Continuous => 1,
        };
        if transaction.execute(
            "UPDATE reader_settings SET fit = ?1, mode = ?2 WHERE id = 1",
            [fit, mode],
        )? != 1
        {
            return Err(LibraryError::InvalidSchema);
        }
        transaction.commit()?;
        Ok(())
    }
}
