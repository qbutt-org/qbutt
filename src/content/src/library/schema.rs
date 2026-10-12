use std::path::Path;

use rusqlite::{Connection, TransactionBehavior};

use super::{Library, LibraryError, LibraryResult};

const APPLICATION_ID: i64 = 0x51424c42;
const SCHEMA_VERSION: i64 = 8;
const DATABASE_NAME: &str = "content-library.sqlite";

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
                ) STRICT;
                CREATE TABLE reader_settings (
                    id INTEGER PRIMARY KEY NOT NULL CHECK (id = 1),
                    fit INTEGER NOT NULL CHECK (fit IN (0, 1)),
                    mode INTEGER NOT NULL CHECK (mode IN (0, 1)),
                    background_color TEXT NOT NULL DEFAULT '',
                    text_color TEXT NOT NULL DEFAULT '',
                    font_family TEXT NOT NULL DEFAULT '',
                    font_size INTEGER NOT NULL DEFAULT 20 CHECK (font_size BETWEEN 12 AND 36),
                    line_height INTEGER NOT NULL DEFAULT 130 CHECK (line_height BETWEEN 110 AND 200),
                    column_width INTEGER NOT NULL DEFAULT 760 CHECK (column_width BETWEEN 480 AND 1040)
                ) STRICT;
                INSERT INTO reader_settings (id, fit, mode) VALUES (1, 0, 0);
                CREATE TABLE source_settings (
                    source TEXT PRIMARY KEY NOT NULL,
                    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
                    revision INTEGER NOT NULL CHECK (revision >= 0),
                    failure TEXT
                ) STRICT;
                CREATE TABLE download_jobs (
                    id TEXT PRIMARY KEY NOT NULL,
                    snapshot TEXT NOT NULL,
                    updated_at INTEGER NOT NULL CHECK (updated_at >= 0)
                ) STRICT;
                CREATE TABLE local_books (
                    publication_id TEXT PRIMARY KEY NOT NULL,
                    path TEXT NOT NULL
                ) STRICT;
                CREATE TABLE translation_context (
                    publication TEXT NOT NULL,
                    language TEXT NOT NULL,
                    revision INTEGER NOT NULL CHECK (revision >= 0),
                    context TEXT NOT NULL,
                    PRIMARY KEY (publication, language)
                ) STRICT;
                CREATE TABLE translated_content (
                    key TEXT PRIMARY KEY NOT NULL,
                    content TEXT NOT NULL
                ) STRICT;",
            )?;
            transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
            transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        } else if application_id != APPLICATION_ID {
            return Err(LibraryError::ForeignDatabase);
        } else if version == 7 {
            transaction.execute_batch(
                "ALTER TABLE reader_settings ADD COLUMN background_color TEXT NOT NULL DEFAULT '';
                ALTER TABLE reader_settings ADD COLUMN text_color TEXT NOT NULL DEFAULT '';
                ALTER TABLE reader_settings ADD COLUMN font_family TEXT NOT NULL DEFAULT '';
                ALTER TABLE reader_settings ADD COLUMN font_size INTEGER NOT NULL DEFAULT 20 CHECK (font_size BETWEEN 12 AND 36);
                ALTER TABLE reader_settings ADD COLUMN line_height INTEGER NOT NULL DEFAULT 130 CHECK (line_height BETWEEN 110 AND 200);
                ALTER TABLE reader_settings ADD COLUMN column_width INTEGER NOT NULL DEFAULT 760 CHECK (column_width BETWEEN 480 AND 1040);
                CREATE TABLE download_jobs (
                    id TEXT PRIMARY KEY NOT NULL,
                    snapshot TEXT NOT NULL,
                    updated_at INTEGER NOT NULL CHECK (updated_at >= 0)
                ) STRICT;
                CREATE TABLE local_books (
                    publication_id TEXT PRIMARY KEY NOT NULL,
                    path TEXT NOT NULL
                ) STRICT;
                CREATE TABLE translation_context (
                    publication TEXT NOT NULL,
                    language TEXT NOT NULL,
                    revision INTEGER NOT NULL CHECK (revision >= 0),
                    context TEXT NOT NULL,
                    PRIMARY KEY (publication, language)
                ) STRICT;
                CREATE TABLE translated_content (
                    key TEXT PRIMARY KEY NOT NULL,
                    content TEXT NOT NULL
                ) STRICT;",
            )?;
            transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        } else if version != SCHEMA_VERSION {
            return Err(LibraryError::UnsupportedSchema(version));
        }
        let table_valid: bool = transaction.query_row(
            "SELECT EXISTS (
                SELECT 1 FROM pragma_table_list
                WHERE schema = 'main' AND name = 'library'
                    AND type = 'table' AND ncol = 3 AND strict = 1
            )",
            [],
            |row| row.get(0),
        )?;
        let columns_valid: i64 = transaction.query_row(
            "SELECT count(*) FROM pragma_table_xinfo('library')
             WHERE hidden = 0 AND \"notnull\" = 1 AND (
                 (cid = 0 AND name = 'reference' AND type = 'TEXT' AND pk = 1)
                 OR (cid = 1 AND name = 'entry' AND type = 'TEXT' AND pk = 0)
                 OR (cid = 2 AND name = 'updated_at' AND type = 'INTEGER' AND pk = 0)
             )",
            [],
            |row| row.get(0),
        )?;
        if !table_valid || columns_valid != 3 {
            return Err(LibraryError::InvalidSchema);
        }
        let settings_table_valid: bool = transaction.query_row(
            "SELECT EXISTS (
                SELECT 1 FROM pragma_table_list
                WHERE schema = 'main' AND name = 'reader_settings'
                    AND type = 'table' AND ncol = 9 AND strict = 1
            )",
            [],
            |row| row.get(0),
        )?;
        let settings_columns_valid: i64 = transaction.query_row(
            "SELECT count(*) FROM pragma_table_xinfo('reader_settings')
             WHERE hidden = 0 AND \"notnull\" = 1 AND (
                 (cid = 0 AND name = 'id' AND type = 'INTEGER' AND pk = 1)
                 OR (cid = 1 AND name = 'fit' AND type = 'INTEGER' AND pk = 0)
                 OR (cid = 2 AND name = 'mode' AND type = 'INTEGER' AND pk = 0)
                 OR (cid = 3 AND name = 'background_color' AND type = 'TEXT' AND pk = 0)
                 OR (cid = 4 AND name = 'text_color' AND type = 'TEXT' AND pk = 0)
                 OR (cid = 5 AND name = 'font_family' AND type = 'TEXT' AND pk = 0)
                 OR (cid = 6 AND name = 'font_size' AND type = 'INTEGER' AND pk = 0)
                 OR (cid = 7 AND name = 'line_height' AND type = 'INTEGER' AND pk = 0)
                 OR (cid = 8 AND name = 'column_width' AND type = 'INTEGER' AND pk = 0)
             )",
            [],
            |row| row.get(0),
        )?;
        if !settings_table_valid || settings_columns_valid != 9 {
            return Err(LibraryError::InvalidSchema);
        }
        let sources_table_valid: bool = transaction.query_row(
            "SELECT EXISTS (
                SELECT 1 FROM pragma_table_list
                WHERE schema = 'main' AND name = 'source_settings'
                    AND type = 'table' AND ncol = 4 AND strict = 1
            )",
            [],
            |row| row.get(0),
        )?;
        let sources_columns_valid: i64 = transaction.query_row(
            "SELECT count(*) FROM pragma_table_xinfo('source_settings')
             WHERE hidden = 0 AND (
                 (cid = 0 AND name = 'source' AND type = 'TEXT' AND pk = 1 AND \"notnull\" = 1)
                 OR (cid = 1 AND name = 'enabled' AND type = 'INTEGER' AND pk = 0 AND \"notnull\" = 1)
                 OR (cid = 2 AND name = 'revision' AND type = 'INTEGER' AND pk = 0 AND \"notnull\" = 1)
                 OR (cid = 3 AND name = 'failure' AND type = 'TEXT' AND pk = 0 AND \"notnull\" = 0)
             )",
            [],
            |row| row.get(0),
        )?;
        if !sources_table_valid || sources_columns_valid != 4 {
            return Err(LibraryError::InvalidSchema);
        }
        let downloads_table_valid: bool = transaction.query_row(
            "SELECT EXISTS (
                SELECT 1 FROM pragma_table_list
                WHERE schema = 'main' AND name = 'download_jobs'
                    AND type = 'table' AND ncol = 3 AND strict = 1
            )",
            [],
            |row| row.get(0),
        )?;
        let downloads_columns_valid: i64 = transaction.query_row(
            "SELECT count(*) FROM pragma_table_xinfo('download_jobs')
             WHERE hidden = 0 AND \"notnull\" = 1 AND (
                 (cid = 0 AND name = 'id' AND type = 'TEXT' AND pk = 1)
                 OR (cid = 1 AND name = 'snapshot' AND type = 'TEXT' AND pk = 0)
                 OR (cid = 2 AND name = 'updated_at' AND type = 'INTEGER' AND pk = 0)
             )",
            [],
            |row| row.get(0),
        )?;
        if !downloads_table_valid || downloads_columns_valid != 3 {
            return Err(LibraryError::InvalidSchema);
        }
        let books_table_valid: bool = transaction.query_row(
            "SELECT EXISTS (
                SELECT 1 FROM pragma_table_list
                WHERE schema = 'main' AND name = 'local_books'
                    AND type = 'table' AND ncol = 2 AND strict = 1
            )",
            [],
            |row| row.get(0),
        )?;
        let books_columns_valid: i64 = transaction.query_row(
            "SELECT count(*) FROM pragma_table_xinfo('local_books')
             WHERE hidden = 0 AND \"notnull\" = 1 AND (
                 (cid = 0 AND name = 'publication_id' AND type = 'TEXT' AND pk = 1)
                 OR (cid = 1 AND name = 'path' AND type = 'TEXT' AND pk = 0)
             )",
            [],
            |row| row.get(0),
        )?;
        if !books_table_valid || books_columns_valid != 2 {
            return Err(LibraryError::InvalidSchema);
        }
        let context_table_valid: bool = transaction.query_row(
            "SELECT EXISTS (
                SELECT 1 FROM pragma_table_list
                WHERE schema = 'main' AND name = 'translation_context'
                    AND type = 'table' AND ncol = 4 AND strict = 1
            )",
            [],
            |row| row.get(0),
        )?;
        let context_columns_valid: i64 = transaction.query_row(
            "SELECT count(*) FROM pragma_table_xinfo('translation_context')
             WHERE hidden = 0 AND \"notnull\" = 1 AND (
                 (cid = 0 AND name = 'publication' AND type = 'TEXT' AND pk = 1)
                 OR (cid = 1 AND name = 'language' AND type = 'TEXT' AND pk = 2)
                 OR (cid = 2 AND name = 'revision' AND type = 'INTEGER' AND pk = 0)
                 OR (cid = 3 AND name = 'context' AND type = 'TEXT' AND pk = 0)
             )",
            [],
            |row| row.get(0),
        )?;
        if !context_table_valid || context_columns_valid != 4 {
            return Err(LibraryError::InvalidSchema);
        }
        let translated_table_valid: bool = transaction.query_row(
            "SELECT EXISTS (
                SELECT 1 FROM pragma_table_list
                WHERE schema = 'main' AND name = 'translated_content'
                    AND type = 'table' AND ncol = 2 AND strict = 1
            )",
            [],
            |row| row.get(0),
        )?;
        let translated_columns_valid: i64 = transaction.query_row(
            "SELECT count(*) FROM pragma_table_xinfo('translated_content')
             WHERE hidden = 0 AND \"notnull\" = 1 AND (
                 (cid = 0 AND name = 'key' AND type = 'TEXT' AND pk = 1)
                 OR (cid = 1 AND name = 'content' AND type = 'TEXT' AND pk = 0)
             )",
            [],
            |row| row.get(0),
        )?;
        if !translated_table_valid || translated_columns_valid != 2 {
            return Err(LibraryError::InvalidSchema);
        }
        // The registered catalog owns the current set. Preserve saved rows and
        // initialize settings for newly registered sources in the same transaction.
        for source in crate::Source::ALL {
            transaction.execute(
                "INSERT OR IGNORE INTO source_settings (source, enabled, revision)
                 VALUES (?1, 1, 0)",
                [source.id()],
            )?;
        }
        transaction.commit()?;
        Ok(Self { connection })
    }
}
