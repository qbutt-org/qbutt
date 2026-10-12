use rusqlite::{TransactionBehavior, params};

use super::{Library, LibraryError, LibraryResult};

/// Profile-wide reading choices, independent of a publication and its progress.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReaderSettings {
    pub fit: ReaderFit,
    pub mode: ReaderMode,
    pub background_color: String,
    pub text_color: String,
    pub font_family: String,
    pub font_size: i32,
    pub line_height: i32,
    pub column_width: i32,
    pub translation_language: String,
}

impl Default for ReaderSettings {
    fn default() -> Self {
        Self {
            fit: ReaderFit::Height,
            mode: ReaderMode::Paged,
            background_color: String::new(),
            text_color: String::new(),
            font_family: String::new(),
            font_size: 20,
            line_height: 130,
            column_width: 760,
            translation_language: "ru".into(),
        }
    }
}

impl ReaderSettings {
    pub fn valid(&self) -> bool {
        fn color_valid(value: &str) -> bool {
            value.is_empty()
                || (value.len() == 7
                    && value.starts_with('#')
                    && value.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit))
        }

        color_valid(&self.background_color)
            && color_valid(&self.text_color)
            && self.font_family.len() <= 256
            && !self.font_family.chars().any(char::is_control)
            && (12..=36).contains(&self.font_size)
            && (110..=200).contains(&self.line_height)
            && (480..=1040).contains(&self.column_width)
            && crate::model::canonical_language(&self.translation_language)
    }
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
        let mut statement = self.connection.prepare(
            "SELECT id, fit, mode, background_color, text_color, font_family,
                      font_size, line_height, column_width, translation_language
               FROM reader_settings",
        )?;
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
        let settings = ReaderSettings {
            fit,
            mode,
            background_color: row.get(3)?,
            text_color: row.get(4)?,
            font_family: row.get(5)?,
            font_size: row.get(6)?,
            line_height: row.get(7)?,
            column_width: row.get(8)?,
            translation_language: row.get(9)?,
        };
        if rows.next()?.is_some() || !settings.valid() {
            return Err(LibraryError::InvalidSchema);
        }
        Ok(settings)
    }

    pub fn save_reader_settings(&mut self, settings: ReaderSettings) -> LibraryResult<()> {
        if !settings.valid() {
            return Err(LibraryError::InvalidSchema);
        }
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
            "UPDATE reader_settings SET fit = ?1, mode = ?2, background_color = ?3,
                text_color = ?4, font_family = ?5, font_size = ?6,
                line_height = ?7, column_width = ?8,
                translation_language = ?9 WHERE id = 1",
            params![
                fit,
                mode,
                settings.background_color,
                settings.text_color,
                settings.font_family,
                settings.font_size,
                settings.line_height,
                settings.column_width,
                settings.translation_language
            ],
        )? != 1
        {
            return Err(LibraryError::InvalidSchema);
        }
        transaction.commit()?;
        Ok(())
    }
}
