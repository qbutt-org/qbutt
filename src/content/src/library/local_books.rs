use std::path::PathBuf;

use rusqlite::{OptionalExtension, params};

use super::{Library, LibraryError, LibraryResult};
use crate::{LocalBook, PublicationRef, Source};

impl Library {
    /// Keep the user's original file in place. Only its verified identity and
    /// private canonical path are stored; neither enters the public Work record.
    pub fn save_local_book(&mut self, book: &LocalBook) -> LibraryResult<()> {
        self.connection.execute(
            "INSERT INTO local_books (publication_id, path) VALUES (?1, ?2)
             ON CONFLICT(publication_id) DO UPDATE SET path = excluded.path",
            params![book.id, book.path().to_string_lossy().as_ref()],
        )?;
        Ok(())
    }

    pub fn local_book_path(&self, reference: &PublicationRef) -> LibraryResult<Option<PathBuf>> {
        if reference.source != Source::LocalBook || reference.validate().is_err() {
            return Err(LibraryError::InvalidEntry("invalid local book reference"));
        }
        let path: Option<String> = self
            .connection
            .query_row(
                "SELECT path FROM local_books WHERE publication_id = ?1",
                [&reference.id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(path.map(PathBuf::from))
    }
}
