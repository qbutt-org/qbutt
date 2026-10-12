//! qcatch's current-user browser connection, owned by the application runtime.
//!
//! Commands and replies are serializable only for the existing foreign wire
//! adapters. Their cookies, headers and acquisition URLs are ephemeral: do not
//! persist them or include them in diagnostics. Public URLs are checked here;
//! browser requests retain qcatch's address policy, and media consumers must
//! validate DNS answers at their actual connection boundary.

mod actor;
mod command;
mod discovery;
mod protocol;
mod reply;
mod security;
mod selection;
mod transport;
mod validation;

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{Notify, Semaphore, mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, timeout_at};
use url::Url;

pub use command::*;
pub use protocol::*;
pub use reply::*;
pub use selection::Selection;

/// Only bounded, public diagnostics cross this boundary. Remote error text and
/// URLs are deliberately discarded because browser exceptions may contain keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BrowserError {
    #[error("Another qbutt profile owns the browser connection")]
    OwnerExists,
    #[error("The browser connection could not be started")]
    StartFailed,
    #[error("Connect qcatch in the selected browser")]
    BrowserRequired,
    #[error("Choose a connected browser")]
    ChoiceRequired,
    #[error("The browser is busy")]
    Busy,
    #[error("Update qcatch to use this browser action")]
    UpdateRequired,
    #[error("The browser request is invalid")]
    InvalidRequest,
    #[error("The browser returned invalid content")]
    InvalidResponse,
    #[error("The browser returned another media selection")]
    SelectionChanged,
    #[error("The selected media is no longer available")]
    SelectionUnavailable,
    #[error("The browser disconnected")]
    Disconnected,
    #[error("The browser request timed out")]
    Timeout,
    #[error("The browser connection stopped")]
    Stopped,
    #[error("The source needs verification")]
    VerificationRequired,
    #[error("Complete the source check in the browser")]
    BrowserCheckRequired,
    #[error("This content needs multiple audio tracks that cannot be forwarded")]
    UnsupportedMultiAudio,
    #[error("This content uses private tracks that cannot be forwarded")]
    UnsupportedPrivateTracks,
    #[error("The selected capture expired. Choose the media again in qcatch.")]
    CaptureExpired,
    #[error("The browser could not resolve this content")]
    SourceUnavailable,
}

impl BrowserError {
    pub fn code(self) -> &'static str {
        match self {
            Self::OwnerExists => "browser_owner_exists",
            Self::StartFailed => "browser_start_failed",
            Self::BrowserRequired => "browser_required",
            Self::ChoiceRequired => "browser_choice_required",
            Self::Busy => "browser_busy",
            Self::UpdateRequired => "browser_update_required",
            Self::InvalidRequest => "browser_invalid_request",
            Self::InvalidResponse => "browser_invalid_response",
            Self::SelectionChanged => "selected_content_changed",
            Self::SelectionUnavailable => "selection_unavailable",
            Self::Disconnected => "browser_disconnected",
            Self::Timeout => "browser_timeout",
            Self::Stopped => "browser_stopped",
            Self::VerificationRequired => "verification_required",
            Self::BrowserCheckRequired => "browser_check_required",
            Self::UnsupportedMultiAudio => "unsupported_multi_audio",
            Self::UnsupportedPrivateTracks => "unsupported_private_tracks",
            Self::CaptureExpired => "capture_expired",
            Self::SourceUnavailable => "source_unavailable",
        }
    }
}

/// The sole owner of discovery and all connection tasks. Drop stops accepting
/// work; use shutdown to wait until clients and discovery are released.
pub struct BrowserBroker {
    handle: BrowserHandle,
    task: Option<JoinHandle<()>>,
}

#[derive(Clone)]
pub struct BrowserHandle {
    requests: mpsc::Sender<actor::Request>,
    status: watch::Receiver<BrowserStatus>,
    stopping: watch::Sender<bool>,
    canceled: Arc<Notify>,
    slots: Arc<Semaphore>,
}

/// Ephemeral native source credentials from one selected browser connection.
/// This value never crosses the foreign wire, Qt models or diagnostics.
pub struct BrowserSession {
    pub(crate) url: Url,
    pub(crate) connection: BrowserConnection,
    pub(crate) cookies: Vec<BrowserCookie>,
}

impl BrowserBroker {
    pub async fn start(discovery_path: PathBuf) -> Result<Self, BrowserError> {
        let mut discovery =
            tokio::task::spawn_blocking(move || discovery::Discovery::open(discovery_path))
                .await
                .map_err(|_| BrowserError::StartFailed)??;
        let listener = security::create_pipe(discovery.pipe_name(), true)
            .map_err(|_| BrowserError::StartFailed)?;
        discovery.publish()?;
        let (requests, incoming) = mpsc::channel(16);
        let (status_tx, status) = watch::channel(BrowserStatus::default());
        let (stopping, stop_rx) = watch::channel(false);
        let canceled = Arc::new(Notify::new());
        let task = tokio::spawn(actor::run(
            discovery,
            listener,
            incoming,
            status_tx,
            stop_rx,
            canceled.clone(),
        ));
        Ok(Self {
            handle: BrowserHandle {
                requests,
                status,
                stopping,
                canceled,
                slots: Arc::new(Semaphore::new(16)),
            },
            task: Some(task),
        })
    }

    pub fn handle(&self) -> BrowserHandle {
        self.handle.clone()
    }

    pub async fn shutdown(mut self) -> Result<(), BrowserError> {
        self.handle.stopping.send_replace(true);
        self.task
            .take()
            .expect("broker task exists")
            .await
            .map_err(|_| BrowserError::Stopped)
    }
}

impl Drop for BrowserBroker {
    fn drop(&mut self) {
        self.handle.stopping.send_replace(true);
    }
}

impl BrowserHandle {
    /// Dropping this future cancels queued or active work. One request occupies
    /// one of sixteen slots until completion, timeout, cancellation or disconnect.
    /// The total deadline includes waiting for qcatch to reconnect: sixty seconds
    /// for ordinary actions and eighty seconds for media resolution.
    pub async fn request(
        &self,
        choice: BrowserChoice,
        command: BrowserCommand,
    ) -> Result<BrowserReply, BrowserError> {
        self.request_inner(choice, command, true)
            .await
            .map(|outcome| outcome.reply)
    }

    /// Submit only to an already connected browser. Optional cookie import
    /// must not delay public-source requests while qcatch reconnects.
    /// Active work is bounded to twenty seconds, or forty-five for resolution.
    pub async fn request_connected(
        &self,
        choice: BrowserChoice,
        command: BrowserCommand,
    ) -> Result<BrowserReply, BrowserError> {
        self.request_inner(choice, command, false)
            .await
            .map(|outcome| outcome.reply)
    }

    /// Import cookies for this exact HTTPS URL without waiting for a browser.
    /// Connection metadata is captured at dispatch, together with the reply's
    /// connection identity, so a concurrent reconnect cannot replace its UA.
    pub async fn session_connected(
        &self,
        choice: BrowserChoice,
        url: Url,
    ) -> Result<BrowserSession, BrowserError> {
        if url.scheme() != "https" {
            return Err(BrowserError::InvalidRequest);
        }
        let outcome = self
            .request_inner(choice, BrowserCommand::Cookies { url: url.clone() }, false)
            .await?;
        let BrowserReply::Cookies(cookies) = outcome.reply else {
            return Err(BrowserError::InvalidResponse);
        };
        Ok(BrowserSession {
            url,
            connection: outcome.connection,
            cookies,
        })
    }

    async fn request_inner(
        &self,
        choice: BrowserChoice,
        command: BrowserCommand,
        wait_for_connection: bool,
    ) -> Result<actor::Outcome, BrowserError> {
        command.validate()?;
        if *self.stopping.borrow() {
            return Err(BrowserError::Stopped);
        }
        let _slot = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| BrowserError::Busy)?;
        let deadline = Instant::now() + command.timeout(wait_for_connection);
        if wait_for_connection {
            let mut status = self.subscribe();
            let mut stopping = self.stopping.subscribe();
            let connected = async {
                loop {
                    let ready = {
                        let snapshot = status.borrow_and_update();
                        if snapshot.stopped || *stopping.borrow() {
                            return Err(BrowserError::Stopped);
                        }
                        snapshot.connections.iter().any(|connection| match choice {
                            BrowserChoice::Auto => true,
                            BrowserChoice::Edge => connection.browser == BrowserId::Edge,
                            BrowserChoice::Chrome => connection.browser == BrowserId::Chrome,
                        })
                    };
                    if ready {
                        return Ok(());
                    }
                    tokio::select! {
                        result = status.changed() => result.map_err(|_| BrowserError::Stopped)?,
                        _ = stopping.changed() => return Err(BrowserError::Stopped),
                    }
                }
            };
            timeout_at(deadline, connected)
                .await
                .map_err(|_| BrowserError::BrowserRequired)??;
        }
        let (reply, response) = oneshot::channel();
        self.requests
            .try_send(actor::Request {
                choice,
                command,
                reply,
                deadline,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => BrowserError::Busy,
                mpsc::error::TrySendError::Closed(_) => BrowserError::Stopped,
            })?;
        let _cancellation = Cancellation(self.canceled.clone());
        response.await.unwrap_or(Err(BrowserError::Stopped))
    }

    pub fn status(&self) -> BrowserStatus {
        self.status.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<BrowserStatus> {
        self.status.clone()
    }
}

struct Cancellation(Arc<Notify>);

impl Drop for Cancellation {
    fn drop(&mut self) {
        self.0.notify_one();
    }
}
