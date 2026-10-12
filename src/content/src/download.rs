//! Native direct-file acquisition. A queue owner supplies the profile's SQLite
//! writes and owns cancellation; this module never persists acquisition URLs.

use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::fs;
use tokio::sync::{Mutex, Semaphore, watch};
use tokio::task::JoinHandle;
use url::Url;

mod transfer;

use crate::http::validate_media_url;
use crate::{LibraryError, LibraryResult, MediaGateway, PreparedPlayback, VariantRef};
use transfer::{
    AddressPolicy, PublishCandidate, local_client, move_without_replace_async, public_client,
    safe_file_name, stage_direct, verify_candidate,
};

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("The selected download is invalid")]
    InvalidSelection,
    #[error("The download address is unsafe")]
    UnsafeAddress,
    #[error("The source returned HTTP {0}")]
    Http(u16),
    #[error("The source redirected too many times")]
    Redirect,
    #[error("The source returned an unsupported file format")]
    Format,
    #[error("This video cannot be saved as one MP4 file")]
    UnsupportedMedia,
    #[error("The source returned an incomplete file")]
    Incomplete,
    #[error("The source returned an empty file")]
    Empty,
    #[error("The download stopped responding")]
    Timeout,
    #[error("The download was stopped")]
    Cancelled,
    #[error("The download queue is closing")]
    Closing,
    #[error("The selected download cannot be retried")]
    Retry,
    #[error("This variant is already downloading")]
    AlreadyQueued,
    #[error("The download could not save its status")]
    Storage(#[from] LibraryError),
    #[error("The download could not access its file")]
    Io(#[from] io::Error),
    #[error("The source could not provide the file")]
    Network(#[source] reqwest::Error),
}

pub struct DirectDownload {
    origin: DownloadOrigin,
    title: String,
    url: Url,
    gateway: Option<MediaGateway>,
}

impl DirectDownload {
    pub fn new(reference: VariantRef, title: String, url: Url) -> Result<Self, DownloadError> {
        reference
            .validate()
            .map_err(|_| DownloadError::InvalidSelection)?;
        validate_media_url(&url).map_err(|_| DownloadError::UnsafeAddress)?;
        if title.trim().is_empty() {
            return Err(DownloadError::InvalidSelection);
        }
        Ok(Self {
            origin: DownloadOrigin::Catalog { reference },
            title,
            url,
            gateway: None,
        })
    }

    pub fn prepared(
        reference: VariantRef,
        title: String,
        prepared: PreparedPlayback,
        gateway: MediaGateway,
    ) -> Result<Self, DownloadError> {
        reference
            .validate()
            .map_err(|_| DownloadError::InvalidSelection)?;
        if title.trim().is_empty() {
            return Err(DownloadError::InvalidSelection);
        }
        let url = prepared
            .muxed_mp4_url()
            .ok_or(DownloadError::UnsupportedMedia)?;
        if !AddressPolicy::Gateway.valid(&url) || url.port() != gateway.proxy_url().port() {
            return Err(DownloadError::UnsafeAddress);
        }
        Ok(Self {
            origin: DownloadOrigin::Catalog { reference },
            title,
            url,
            gateway: Some(gateway),
        })
    }

    pub fn captured(
        source_host: String,
        title: String,
        prepared: PreparedPlayback,
        gateway: MediaGateway,
    ) -> Result<Self, DownloadError> {
        let origin = DownloadOrigin::Browser { source_host };
        origin.validate()?;
        if title.trim().is_empty() {
            return Err(DownloadError::InvalidSelection);
        }
        let url = prepared
            .muxed_mp4_url()
            .ok_or(DownloadError::UnsupportedMedia)?;
        if !AddressPolicy::Gateway.valid(&url) || url.port() != gateway.proxy_url().port() {
            return Err(DownloadError::UnsafeAddress);
        }
        Ok(Self {
            origin,
            title,
            url,
            gateway: Some(gateway),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DownloadOrigin {
    Catalog { reference: VariantRef },
    Browser { source_host: String },
}

impl DownloadOrigin {
    fn interrupted_message(&self) -> &'static str {
        match self {
            Self::Catalog { .. } => "Download interrupted. Resolve the original variant to retry.",
            Self::Browser { .. } => "Download interrupted. Choose the video again in qcatch.",
        }
    }

    fn validate(&self) -> Result<(), DownloadError> {
        match self {
            Self::Catalog { reference } => reference
                .validate()
                .map_err(|_| DownloadError::InvalidSelection),
            Self::Browser { source_host } => {
                if source_host.len() > 253
                    || source_host.len() < 4
                    || !source_host.contains('.')
                    || !source_host.bytes().all(|byte| {
                        byte.is_ascii_lowercase()
                            || byte.is_ascii_digit()
                            || matches!(byte, b'.' | b'-')
                    })
                {
                    return Err(DownloadError::InvalidSelection);
                }
                Ok(())
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadStatus {
    Queued,
    Downloading,
    Publishing,
    Complete,
    Cancelled,
    Error,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DownloadJob {
    pub id: String,
    pub title: String,
    pub origin: DownloadOrigin,
    pub status: DownloadStatus,
    pub bytes: u64,
    pub total_bytes: Option<u64>,
    pub path: Option<PathBuf>,
    pub error: Option<String>,
    candidate: Option<PublishCandidate>,
}

impl DownloadJob {
    fn queued(request: &DirectDownload) -> Result<Self, DownloadError> {
        let mut id = [0u8; 16];
        getrandom::fill(&mut id).map_err(|_| io::Error::other("download ID unavailable"))?;
        Ok(Self {
            id: id.iter().map(|byte| format!("{byte:02x}")).collect(),
            title: request.title.clone(),
            origin: request.origin.clone(),
            status: DownloadStatus::Queued,
            bytes: 0,
            total_bytes: None,
            path: None,
            error: None,
            candidate: None,
        })
    }

    fn is_active(&self) -> bool {
        matches!(
            self.status,
            DownloadStatus::Queued | DownloadStatus::Downloading | DownloadStatus::Publishing
        )
    }

    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        if self.id.len() != 32 || !self.id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("download ID");
        }
        if self.title.trim().is_empty() {
            return Err("download title");
        }
        self.origin.validate().map_err(|_| "download origin")?;
        Ok(())
    }
}

/// Implement this on the existing profile's sole SQLite worker. Its futures
/// acknowledge committed writes before the queue changes visible job state.
pub trait DownloadStorage: Send + Sync + 'static {
    fn load(&self) -> impl Future<Output = LibraryResult<Vec<DownloadJob>>> + Send;
    fn save(&self, job: DownloadJob) -> impl Future<Output = LibraryResult<()>> + Send;
}

struct QueueState {
    jobs: HashMap<String, DownloadJob>,
    cancellation: HashMap<String, watch::Sender<bool>>,
    closed: bool,
}

struct QueueInner<S: DownloadStorage> {
    storage: S,
    destination: PathBuf,
    client: Client,
    local_client: Client,
    state: Mutex<QueueState>,
    sequential: Semaphore,
    updates: watch::Sender<Vec<DownloadJob>>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

pub struct DownloadQueue<S: DownloadStorage> {
    inner: Arc<QueueInner<S>>,
}

impl<S: DownloadStorage> DownloadQueue<S> {
    pub async fn open(storage: S, destination: PathBuf) -> Result<Self, DownloadError> {
        Self::open_with_client(storage, destination, public_client()?).await
    }

    #[cfg(feature = "download-fixture")]
    pub async fn open_fixture(
        storage: S,
        destination: PathBuf,
        client: Client,
    ) -> Result<Self, DownloadError> {
        Self::open_with_client(storage, destination, client).await
    }

    async fn open_with_client(
        storage: S,
        destination: PathBuf,
        client: Client,
    ) -> Result<Self, DownloadError> {
        if !destination.is_absolute() {
            return Err(DownloadError::Io(io::ErrorKind::InvalidInput.into()));
        }
        fs::create_dir_all(&destination).await?;
        let staging = destination.join(".qbutt-staging");
        fs::create_dir_all(&staging).await?;
        let metadata = fs::symlink_metadata(&staging).await?;
        if !metadata.is_dir() || staging_is_reparse_point(&metadata) {
            return Err(DownloadError::Io(io::ErrorKind::PermissionDenied.into()));
        }
        let mut records = HashMap::new();
        for mut job in storage.load().await? {
            job.validate()
                .map_err(|_| DownloadError::Storage(LibraryError::CorruptEntry("download job")))?;
            let original = job.status;
            let interrupted = job.origin.interrupted_message();
            if job.status == DownloadStatus::Publishing {
                let completed = match &job.candidate {
                    Some(candidate) if candidate.path.parent() == Some(destination.as_path()) => {
                        verify_candidate(candidate).await?
                    }
                    _ => false,
                };
                if completed {
                    job.path = job
                        .candidate
                        .as_ref()
                        .map(|candidate| candidate.path.clone());
                    job.status = DownloadStatus::Complete;
                    job.error = None;
                } else {
                    job.status = DownloadStatus::Error;
                    job.error = Some(interrupted.into());
                }
            } else if job.is_active() {
                job.status = DownloadStatus::Error;
                job.error = Some(interrupted.into());
            } else if job.status == DownloadStatus::Complete
                && !job.path.as_ref().is_some_and(|path| path.is_file())
            {
                job.status = DownloadStatus::Error;
                job.error = Some("The downloaded file is no longer available.".into());
            }
            job.candidate = None;
            remove_partial(&destination, &job.id).await?;
            if job.status != original || original == DownloadStatus::Publishing {
                storage.save(job.clone()).await?;
            }
            records.insert(job.id.clone(), job);
        }
        let initial = ordered_jobs(&records);
        let (updates, _) = watch::channel(initial);
        Ok(Self {
            inner: Arc::new(QueueInner {
                storage,
                destination,
                client,
                local_client: local_client()?,
                state: Mutex::new(QueueState {
                    jobs: records,
                    cancellation: HashMap::new(),
                    closed: false,
                }),
                sequential: Semaphore::new(1),
                updates,
                tasks: Mutex::new(Vec::new()),
            }),
        })
    }

    pub fn subscribe(&self) -> watch::Receiver<Vec<DownloadJob>> {
        self.inner.updates.subscribe()
    }

    pub async fn enqueue(&self, request: DirectDownload) -> Result<DownloadJob, DownloadError> {
        let job = DownloadJob::queued(&request)?;
        let mut state = self.inner.state.lock().await;
        if state.closed {
            return Err(DownloadError::Closing);
        }
        if state.jobs.values().any(|current| {
            matches!(&request.origin, DownloadOrigin::Catalog { .. })
                && current.origin == request.origin
                && current.is_active()
        }) {
            return Err(DownloadError::AlreadyQueued);
        }
        self.inner.storage.save(job.clone()).await?;
        let (cancel, receiver) = watch::channel(false);
        state.cancellation.insert(job.id.clone(), cancel);
        state.jobs.insert(job.id.clone(), job.clone());
        self.inner.updates.send_replace(ordered_jobs(&state.jobs));
        self.spawn(job.id.clone(), request, receiver).await;
        drop(state);
        Ok(job)
    }

    pub async fn retry(&self, id: &str, request: DirectDownload) -> Result<(), DownloadError> {
        let mut state = self.inner.state.lock().await;
        if state.closed {
            return Err(DownloadError::Closing);
        }
        let Some(saved) = state.jobs.get(id) else {
            return Err(DownloadError::Retry);
        };
        if !matches!(
            saved.status,
            DownloadStatus::Cancelled | DownloadStatus::Error
        ) || saved.origin != request.origin
            || !matches!(&saved.origin, DownloadOrigin::Catalog { .. })
        {
            return Err(DownloadError::Retry);
        }
        if state.jobs.values().any(|current| {
            current.id != id && current.origin == request.origin && current.is_active()
        }) {
            return Err(DownloadError::AlreadyQueued);
        }
        let mut job = saved.clone();
        job.status = DownloadStatus::Queued;
        job.bytes = 0;
        job.total_bytes = None;
        job.path = None;
        job.error = None;
        job.candidate = None;
        self.inner.storage.save(job.clone()).await?;
        let (cancel, receiver) = watch::channel(false);
        state.cancellation.insert(id.to_owned(), cancel);
        state.jobs.insert(id.to_owned(), job);
        self.inner.updates.send_replace(ordered_jobs(&state.jobs));
        self.spawn(id.to_owned(), request, receiver).await;
        drop(state);
        Ok(())
    }

    pub async fn cancel(&self, id: &str) -> Result<(), DownloadError> {
        let state = self.inner.state.lock().await;
        if state.jobs.get(id).is_some_and(|job| {
            matches!(
                job.status,
                DownloadStatus::Queued | DownloadStatus::Downloading
            )
        }) {
            if let Some(cancel) = state.cancellation.get(id) {
                cancel.send_replace(true);
            }
        }
        Ok(())
    }

    pub async fn close(&self) {
        let mut state = self.inner.state.lock().await;
        state.closed = true;
        for cancel in state.cancellation.values() {
            cancel.send_replace(true);
        }
        drop(state);
        for task in self.inner.tasks.lock().await.drain(..) {
            let _ = task.await;
        }
    }

    async fn spawn(&self, id: String, request: DirectDownload, cancel: watch::Receiver<bool>) {
        let inner = Arc::clone(&self.inner);
        let task = tokio::spawn(async move {
            let outcome = Arc::clone(&inner).run(&id, request, cancel).await;
            if let Err(error) = inner.finish(&id, outcome).await {
                eprintln!("qbutt download state: {error}");
            }
        });
        let mut tasks = self.inner.tasks.lock().await;
        tasks.retain(|task| !task.is_finished());
        tasks.push(task);
    }
}

impl<S: DownloadStorage> QueueInner<S> {
    async fn update(
        &self,
        id: &str,
        change: impl FnOnce(&mut DownloadJob),
    ) -> Result<(), DownloadError> {
        let mut state = self.state.lock().await;
        let mut job = state.jobs.get(id).ok_or(DownloadError::Retry)?.clone();
        change(&mut job);
        self.storage.save(job.clone()).await?;
        state.jobs.insert(id.to_owned(), job);
        self.updates.send_replace(ordered_jobs(&state.jobs));
        Ok(())
    }

    async fn finish(
        &self,
        id: &str,
        outcome: Result<PathBuf, DownloadError>,
    ) -> Result<(), DownloadError> {
        let outcome = match remove_partial(&self.destination, id).await {
            Ok(()) => outcome,
            Err(error) => Err(error),
        };
        let state = self.state.lock().await;
        let cancelled = state
            .cancellation
            .get(id)
            .is_some_and(|sender| *sender.borrow());
        let closing = state.closed;
        drop(state);
        let result = self
            .update(id, |job| {
                if job.status != DownloadStatus::Complete {
                    match outcome {
                        Ok(path) => {
                            job.path = Some(path);
                            job.status = DownloadStatus::Complete;
                            job.error = None;
                        }
                        Err(error) => {
                            if cancelled && !closing {
                                job.status = DownloadStatus::Cancelled;
                                job.error = None;
                            } else {
                                job.status = DownloadStatus::Error;
                                job.error = Some(if closing {
                                    job.origin.interrupted_message().into()
                                } else {
                                    error.to_string()
                                });
                            }
                        }
                    }
                    job.candidate = None;
                }
            })
            .await;
        self.state.lock().await.cancellation.remove(id);
        result
    }

    async fn run(
        self: Arc<Self>,
        id: &str,
        mut request: DirectDownload,
        cancel: watch::Receiver<bool>,
    ) -> Result<PathBuf, DownloadError> {
        let gateway = request.gateway.take();
        let policy = if gateway.is_some() {
            AddressPolicy::Gateway
        } else {
            AddressPolicy::Public
        };
        let result = self.run_transfer(id, &request, policy, cancel).await;
        if let Some(gateway) = gateway {
            gateway.close().await;
        }
        result
    }

    async fn run_transfer(
        &self,
        id: &str,
        request: &DirectDownload,
        policy: AddressPolicy,
        mut cancel: watch::Receiver<bool>,
    ) -> Result<PathBuf, DownloadError> {
        let _permit = self
            .sequential
            .acquire()
            .await
            .map_err(|_| DownloadError::Closing)?;
        if *cancel.borrow() {
            return Err(DownloadError::Cancelled);
        }
        self.update(id, |job| job.status = DownloadStatus::Downloading)
            .await?;
        let partial = partial_path(&self.destination, id);
        let (progress_sender, mut progress_receiver) = watch::channel((0, None));
        let mut last_reported = 0;
        let mut last_report = Instant::now();
        let client = match policy {
            AddressPolicy::Public => &self.client,
            AddressPolicy::Gateway => &self.local_client,
        };
        let transfer = stage_direct(client, &request.url, policy, &partial, |bytes, total| {
            if last_reported == 0
                || bytes.saturating_sub(last_reported) >= 4 * 1024 * 1024
                || last_report.elapsed() >= Duration::from_millis(250)
            {
                progress_sender.send_replace((bytes, total));
                last_reported = bytes;
                last_report = Instant::now();
            }
        });
        tokio::pin!(transfer);
        let staged = loop {
            tokio::select! {
                result = &mut transfer => break result?,
                _ = cancel.changed() => {
                    if *cancel.borrow() { return Err(DownloadError::Cancelled); }
                }
                Ok(()) = progress_receiver.changed() => {
                    let (bytes, total) = *progress_receiver.borrow();
                    let mut state = self.state.lock().await;
                    if let Some(job) = state.jobs.get_mut(id) {
                        job.bytes = bytes;
                        job.total_bytes = total;
                    }
                    self.updates.send_replace(ordered_jobs(&state.jobs));
                }
            }
        };
        if *cancel.borrow() {
            staged.discard().await?;
            return Err(DownloadError::Cancelled);
        }
        let name = safe_file_name(&request.title);
        let stem = &name[..name.len() - 4];
        for index in 1..=10_000 {
            if *cancel.borrow() {
                staged.discard().await?;
                return Err(DownloadError::Cancelled);
            }
            let candidate = self.destination.join(if index == 1 {
                name.clone()
            } else {
                format!("{stem} ({index}).mp4")
            });
            let journal = staged.candidate(candidate.clone());
            let mut state = self.state.lock().await;
            if *cancel.borrow() {
                drop(state);
                staged.discard().await?;
                return Err(DownloadError::Cancelled);
            }
            let mut job = state.jobs.get(id).ok_or(DownloadError::Retry)?.clone();
            job.status = DownloadStatus::Publishing;
            job.bytes = staged.size();
            job.total_bytes = Some(staged.size());
            job.candidate = Some(journal);
            self.storage.save(job.clone()).await?;
            state.jobs.insert(id.to_owned(), job);
            self.updates.send_replace(ordered_jobs(&state.jobs));
            drop(state);
            match move_without_replace_async(&partial, &candidate).await {
                Ok(()) => return Ok(candidate),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(DownloadError::Io(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "download name exhausted",
        )))
    }
}

fn ordered_jobs(jobs: &HashMap<String, DownloadJob>) -> Vec<DownloadJob> {
    let mut jobs = jobs.values().cloned().collect::<Vec<_>>();
    jobs.sort_by(|left, right| left.id.cmp(&right.id));
    jobs
}

fn partial_path(destination: &Path, id: &str) -> PathBuf {
    destination
        .join(".qbutt-staging")
        .join(format!("{id}.part"))
}

#[cfg(windows)]
fn staging_is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn staging_is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

async fn remove_partial(destination: &Path, id: &str) -> Result<(), DownloadError> {
    match fs::remove_file(partial_path(destination, id)).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}
