use rusqlite::params;

use super::{Library, LibraryError, LibraryResult, now_millis};
use crate::DownloadJob;

impl Library {
    pub fn download_jobs(&self) -> LibraryResult<Vec<DownloadJob>> {
        let mut statement = self
            .connection
            .prepare("SELECT id, snapshot FROM download_jobs ORDER BY updated_at DESC, id")?;
        let mut rows = statement.query([])?;
        let mut jobs = Vec::new();
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            let job: DownloadJob = serde_json::from_str(&row.get::<_, String>(1)?)?;
            job.validate().map_err(LibraryError::CorruptEntry)?;
            if job.id != id {
                return Err(LibraryError::CorruptEntry("download row identity"));
            }
            jobs.push(job);
        }
        Ok(jobs)
    }

    pub fn save_download_job(&mut self, job: &DownloadJob) -> LibraryResult<()> {
        job.validate().map_err(LibraryError::InvalidEntry)?;
        self.connection.execute(
            "INSERT INTO download_jobs (id, snapshot, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (id) DO UPDATE SET
                 snapshot = excluded.snapshot,
                 updated_at = excluded.updated_at",
            params![job.id, serde_json::to_string(job)?, now_millis()?],
        )?;
        Ok(())
    }
}
