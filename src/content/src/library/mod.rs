//! Synchronous profile storage. Its caller owns scheduling outside the GUI
//! thread; this module creates no runtime, worker, network request or process.

mod entry;

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::{PublicationRef, VariantRef, Work};

pub use entry::{LastOpened, LibraryEntry, VariantFilters, VariantPreferences};

const APPLICATION_ID: i64 = 0x51424c42;
const SCHEMA_VERSION: i64 = 1;
const DATABASE_NAME: &str = "content-library.sqlite";

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("The content library could not be opened")]
    Io(#[from] std::io::Error),
    #[error("The content library database operation failed")]
    Sqlite(#[from] rusqlite::Error),
    #[error("This file is not a qbutt content library")]
    ForeignDatabase,
    #[error("Content library schema {0} is unsupported")]
    UnsupportedSchema(i64),
    #[error("The content library contains invalid saved metadata")]
    Metadata(#[from] serde_json::Error),
    #[error("The content library contains a corrupt entry: {0}")]
    CorruptEntry(&'static str),
    #[error("The content library update was rejected: {0}")]
    InvalidEntry(&'static str),
    #[error("The system clock cannot provide a valid library timestamp")]
    Clock,
}

pub type LibraryResult<T> = std::result::Result<T, LibraryError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryView {
    All,
    Favorites,
    History,
}

/// One owner of a native SQLite connection for a caller-selected profile.
/// Drop the library to close it. No legacy database is read or modified.
pub struct Library {
    connection: Connection,
}

impl Library {
    /// Open `content-library.sqlite` inside this profile directory. A new schema
    /// is created only in an empty database; unsupported or corrupt data fails.
    pub fn open(profile_dir: impl AsRef<Path>) -> LibraryResult<Self> {
        std::fs::create_dir_all(&profile_dir)?;
        let mut connection = Connection::open(profile_dir.as_ref().join(DATABASE_NAME))?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let application_id: i64 =
            transaction.pragma_query_value(None, "application_id", |row| row.get(0))?;
        let version: i64 =
            transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;

        if application_id == 0 && version == 0 {
            let objects: i64 = transaction.query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
                [],
                |row| row.get(0),
            )?;
            if objects != 0 {
                return Err(LibraryError::ForeignDatabase);
            }
            transaction.execute_batch(
                "CREATE TABLE library (
                    reference TEXT PRIMARY KEY NOT NULL,
                    entry TEXT NOT NULL,
                    updated_at INTEGER NOT NULL CHECK (updated_at >= 0)
                ) STRICT;",
            )?;
            transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
            transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        } else if application_id != APPLICATION_ID {
            return Err(LibraryError::ForeignDatabase);
        } else if version != SCHEMA_VERSION {
            return Err(LibraryError::UnsupportedSchema(version));
        }
        transaction.prepare("SELECT reference, entry, updated_at FROM library LIMIT 0")?;
        transaction.commit()?;
        Ok(Self { connection })
    }

    pub fn get(&self, reference: &PublicationRef) -> LibraryResult<Option<LibraryEntry>> {
        read_entry(&self.connection, reference)
    }

    /// History is ordered by successful action time. Other views use the last
    /// saved change; saved preferences alone do not create a history entry.
    pub fn entries(&self, view: LibraryView) -> LibraryResult<Vec<LibraryEntry>> {
        let mut statement = self
            .connection
            .prepare("SELECT reference, entry FROM library ORDER BY updated_at DESC, reference")?;
        let mut rows = statement.query([])?;
        let mut entries = Vec::new();
        while let Some(row) = rows.next()? {
            let key: String = row.get(0)?;
            let entry = decode_entry(&key, &row.get::<_, String>(1)?)?;
            if match view {
                LibraryView::All => true,
                LibraryView::Favorites => entry.favorite,
                LibraryView::History => entry.last_opened.is_some(),
            } {
                entries.push(entry);
            }
        }
        if view == LibraryView::History {
            entries.sort_by_key(|entry| {
                std::cmp::Reverse(entry.last_opened.as_ref().unwrap().at_unix_millis)
            });
        }
        Ok(entries)
    }

    pub fn set_favorite(&mut self, work: &Work, favorite: bool) -> LibraryResult<LibraryEntry> {
        self.update(work, |entry, _| entry.favorite = favorite)
    }

    /// Save dropdowns and exact selection without recording an opened item.
    pub fn save_preferences(
        &mut self,
        work: &Work,
        preferences: VariantPreferences,
    ) -> LibraryResult<LibraryEntry> {
        self.update(work, |entry, _| entry.preferences = preferences)
    }

    /// Call only after the actual content action succeeds, never on selection
    /// or resolution alone. The exact opened variant and current choices are
    /// committed atomically with the full known work.
    pub fn record_opened(
        &mut self,
        work: &Work,
        variant: VariantRef,
        preferences: VariantPreferences,
    ) -> LibraryResult<LibraryEntry> {
        self.update(work, |entry, at_unix_millis| {
            entry.preferences = preferences;
            entry.last_opened = Some(LastOpened {
                variant,
                at_unix_millis,
            });
        })
    }

    /// Removing history retains the favorite, exact selection and known work.
    /// It never removes downloaded payload or any other profile data.
    pub fn remove_history(
        &mut self,
        reference: &PublicationRef,
    ) -> LibraryResult<Option<LibraryEntry>> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(mut entry) = read_entry(&transaction, reference)? else {
            return Ok(None);
        };
        entry.last_opened = None;
        write_entry(&transaction, &entry, now_millis()?)?;
        transaction.commit()?;
        Ok(Some(entry))
    }

    fn update(
        &mut self,
        work: &Work,
        change: impl FnOnce(&mut LibraryEntry, i64),
    ) -> LibraryResult<LibraryEntry> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut entry = match read_entry(&transaction, &work.reference)? {
            Some(mut entry) => {
                entry.refresh(work);
                entry
            }
            None => LibraryEntry {
                work: work.clone(),
                preferences: VariantPreferences::default(),
                favorite: false,
                last_opened: None,
            },
        };
        let timestamp = now_millis()?;
        change(&mut entry, timestamp);
        write_entry(&transaction, &entry, timestamp)?;
        transaction.commit()?;
        Ok(entry)
    }
}

fn read_entry(
    connection: &Connection,
    reference: &PublicationRef,
) -> LibraryResult<Option<LibraryEntry>> {
    let key = serde_json::to_string(reference)?;
    let metadata: Option<String> = connection
        .query_row(
            "SELECT entry FROM library WHERE reference = ?1",
            [&key],
            |row| row.get(0),
        )
        .optional()?;
    metadata
        .map(|metadata| decode_entry(&key, &metadata))
        .transpose()
}

fn decode_entry(key: &str, metadata: &str) -> LibraryResult<LibraryEntry> {
    let entry: LibraryEntry = serde_json::from_str(metadata)?;
    entry.validate().map_err(LibraryError::CorruptEntry)?;
    if serde_json::to_string(&entry.work.reference)? != key {
        return Err(LibraryError::CorruptEntry(
            "the row identity does not match its work",
        ));
    }
    Ok(entry)
}

fn write_entry(connection: &Connection, entry: &LibraryEntry, timestamp: i64) -> LibraryResult<()> {
    entry.validate().map_err(LibraryError::InvalidEntry)?;
    connection.execute(
        "INSERT INTO library (reference, entry, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT (reference) DO UPDATE SET
             entry = excluded.entry,
             updated_at = excluded.updated_at",
        params![
            serde_json::to_string(&entry.work.reference)?,
            serde_json::to_string(entry)?,
            timestamp,
        ],
    )?;
    Ok(())
}

fn now_millis() -> LibraryResult<i64> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| LibraryError::Clock)?;
    i64::try_from(elapsed.as_millis()).map_err(|_| LibraryError::Clock)
}
