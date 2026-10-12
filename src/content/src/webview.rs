//! The existing WebView2 helper's private, bounded source-check protocol.
//! The helper owns its STA and origin profile; this owner supplies the guarded
//! network path and retires the process when the last caller leaves.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::watch;
use tokio::task::AbortHandle;
use url::Url;
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

use crate::MediaGateway;
use crate::http::validate_media_url;

const MAX_REQUEST_BYTES: usize = 16 * 1024;
const MAX_RESPONSE_BYTES: u64 = 256 * 1024;
const SILENT_TIMEOUT_MS: u32 = 15_000;
const INTERACTIVE_TIMEOUT_MS: u32 = 120_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WebViewError {
    #[error("The source check request is invalid")]
    InvalidRequest,
    #[error("The source check component is unavailable")]
    Unavailable,
    #[error("The source check failed")]
    Failed,
    #[error("The source check timed out")]
    Timeout,
    #[error("The source needs a manual check")]
    CheckRequired,
    #[error("The source check was canceled")]
    Canceled,
    #[error("The source check has stopped")]
    Stopped,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebViewCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires: Option<f64>,
    pub secure: bool,
    pub http_only: bool,
    pub host_only: bool,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebViewCookieIdentity {
    pub name: String,
    pub domain: String,
    pub path: String,
    pub host_only: bool,
}

#[derive(Default)]
pub struct WebViewCheck {
    /// The configured authorization identity. Never replace this with a hash
    /// of refreshed clearance cookies or a profile reset would erase them.
    pub configured_session: Option<String>,
    pub initial_cookies: Vec<WebViewCookie>,
    pub authoritative_cookies: bool,
    pub retained_cookies: Vec<WebViewCookieIdentity>,
    /// Only a user action may set this; ordinary challenge recovery is silent.
    pub interactive: bool,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebViewSession {
    pub url: Url,
    pub user_agent: String,
    pub cookies: Vec<WebViewCookie>,
}

struct Pending {
    state: String,
    result: watch::Sender<Option<Result<Arc<WebViewSession>, WebViewError>>>,
    users: AtomicUsize,
    task: Mutex<Option<AbortHandle>>,
    origin: String,
    owner: Weak<Inner>,
}

struct Subscription(Arc<Pending>);

impl Drop for Subscription {
    fn drop(&mut self) {
        let pending = &self.0;
        if pending.users.fetch_sub(1, Ordering::AcqRel) != 1 {
            return;
        }
        pending
            .result
            .send_replace(Some(Err(WebViewError::Canceled)));
        if let Some(owner) = pending.owner.upgrade() {
            let mut entries = owner.pending.lock().expect("WebView pending lock");
            if entries
                .get(&pending.origin)
                .is_some_and(|entry| Arc::ptr_eq(entry, pending))
            {
                entries.remove(&pending.origin);
            }
        }
        if let Some(task) = pending.task.lock().expect("WebView task lock").take() {
            task.abort();
        }
    }
}

struct Inner {
    data_dir: PathBuf,
    app_dir: PathBuf,
    pending: Mutex<HashMap<String, Arc<Pending>>>,
    stopped: AtomicBool,
}

pub struct WebViewSessions(Arc<Inner>);

impl WebViewSessions {
    pub fn new(data_dir: PathBuf, app_dir: PathBuf) -> Self {
        Self(Arc::new(Inner {
            data_dir,
            app_dir,
            pending: Mutex::new(HashMap::new()),
            stopped: AtomicBool::new(false),
        }))
    }

    pub async fn check(
        &self,
        url: Url,
        options: WebViewCheck,
    ) -> Result<WebViewSession, WebViewError> {
        validate_media_url(&url).map_err(|_| WebViewError::InvalidRequest)?;
        let origin = url.origin().ascii_serialization();
        let mut seeds = options.initial_cookies.clone();
        seeds.sort_by(|left, right| left.name.cmp(&right.name));
        let mut retained = options.retained_cookies.clone();
        retained.sort_by(|left, right| left.name.cmp(&right.name));
        let fingerprint = options
            .configured_session
            .clone()
            .unwrap_or_else(|| digest(&serde_json::to_vec(&seeds).unwrap()));
        if !hex_digest(&fingerprint)
            || options.initial_cookies.len() > 128
            || options.retained_cookies.len() > 128
        {
            return Err(WebViewError::InvalidRequest);
        }
        let state = digest(
            &serde_json::to_vec(&(
                &fingerprint,
                &seeds,
                options.authoritative_cookies,
                &retained,
                options.interactive,
            ))
            .unwrap(),
        );
        loop {
            if self.0.stopped.load(Ordering::Acquire) {
                return Err(WebViewError::Stopped);
            }
            let existing = {
                self.0
                    .pending
                    .lock()
                    .expect("WebView pending lock")
                    .get(&origin)
                    .cloned()
            };
            if let Some(existing) = existing {
                let mut result = existing.result.subscribe();
                if existing.state == state && result.borrow().is_none() {
                    existing.users.fetch_add(1, Ordering::AcqRel);
                    let subscription = Subscription(existing);
                    let answer = wait_result(&mut result, options.interactive).await;
                    drop(subscription);
                    return answer.map(|session| (*session).clone());
                }
                if result.borrow().is_none() {
                    let _ = result.changed().await;
                }
                continue;
            }
            let (sender, mut receiver) = watch::channel(None);
            let pending = Arc::new(Pending {
                state: state.clone(),
                result: sender,
                users: AtomicUsize::new(1),
                task: Mutex::new(None),
                origin: origin.clone(),
                owner: Arc::downgrade(&self.0),
            });
            {
                let mut entries = self.0.pending.lock().expect("WebView pending lock");
                if entries.contains_key(&origin) {
                    continue;
                }
                entries.insert(origin.clone(), pending.clone());
            }
            let owner = self.0.clone();
            let current = pending.clone();
            let origin_key = origin.clone();
            let interactive = options.interactive;
            let task = tokio::spawn(async move {
                let answer = run(&owner, url, &origin_key, &fingerprint, options).await;
                current.result.send_replace(Some(answer.map(Arc::new)));
                let mut entries = owner.pending.lock().expect("WebView pending lock");
                if entries
                    .get(&origin_key)
                    .is_some_and(|entry| Arc::ptr_eq(entry, &current))
                {
                    entries.remove(&origin_key);
                }
            });
            *pending.task.lock().expect("WebView task lock") = Some(task.abort_handle());
            let subscription = Subscription(pending);
            let answer = wait_result(&mut receiver, interactive).await;
            drop(subscription);
            return answer.map(|session| (*session).clone());
        }
    }

    pub fn close(&self) {
        self.0.stopped.store(true, Ordering::Release);
        let pending = std::mem::take(&mut *self.0.pending.lock().expect("WebView pending lock"));
        for entry in pending.into_values() {
            entry.result.send_replace(Some(Err(WebViewError::Stopped)));
            if let Some(task) = entry.task.lock().expect("WebView task lock").take() {
                task.abort();
            }
        }
    }
}

impl Drop for WebViewSessions {
    fn drop(&mut self) {
        self.close();
    }
}

async fn wait_result(
    receiver: &mut watch::Receiver<Option<Result<Arc<WebViewSession>, WebViewError>>>,
    interactive: bool,
) -> Result<Arc<WebViewSession>, WebViewError> {
    let timeout_ms = if interactive {
        INTERACTIVE_TIMEOUT_MS
    } else {
        SILENT_TIMEOUT_MS
    };
    tokio::time::timeout(
        Duration::from_millis(u64::from(timeout_ms) + 5_000),
        async {
            loop {
                if let Some(result) = receiver.borrow_and_update().clone() {
                    return result;
                }
                receiver
                    .changed()
                    .await
                    .map_err(|_| WebViewError::Stopped)?;
            }
        },
    )
    .await
    .map_err(|_| WebViewError::Timeout)?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProxyRequest {
    url: String,
    username: String,
    password: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HelperRequest<'a> {
    id: String,
    url: &'a Url,
    profile_directory: String,
    mode: &'static str,
    timeout_ms: u32,
    configured_session: &'a str,
    initial_cookies: &'a [WebViewCookie],
    authoritative_cookies: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    retained_cookies: Option<&'a [WebViewCookieIdentity]>,
    proxy: ProxyRequest,
}

#[derive(Deserialize)]
struct HelperResponse {
    id: String,
    session: Option<WebViewSession>,
    error: Option<HelperError>,
}

#[derive(Deserialize)]
struct HelperError {
    code: String,
}

async fn run(
    owner: &Inner,
    url: Url,
    origin: &str,
    fingerprint: &str,
    options: WebViewCheck,
) -> Result<WebViewSession, WebViewError> {
    let gateway = MediaGateway::start()
        .await
        .map_err(|_| WebViewError::Failed)?;
    let result = run_with_proxy(
        owner,
        url,
        origin,
        fingerprint,
        options,
        gateway.proxy_url(),
    )
    .await;
    gateway.close().await;
    result
}

async fn run_with_proxy(
    owner: &Inner,
    url: Url,
    origin: &str,
    fingerprint: &str,
    options: WebViewCheck,
    proxy: &Url,
) -> Result<WebViewSession, WebViewError> {
    let mut id_bytes = [0u8; 16];
    getrandom::fill(&mut id_bytes).map_err(|_| WebViewError::Failed)?;
    let id = hex(&id_bytes);
    let profile = owner
        .data_dir
        .join("web-session")
        .join(digest(origin.as_bytes()));
    let timeout_ms = if options.interactive {
        INTERACTIVE_TIMEOUT_MS
    } else {
        SILENT_TIMEOUT_MS
    };
    let request = HelperRequest {
        id: id.clone(),
        url: &url,
        profile_directory: profile.to_string_lossy().into_owned(),
        mode: if options.interactive {
            "interactive"
        } else {
            "silent"
        },
        timeout_ms,
        configured_session: fingerprint,
        initial_cookies: &options.initial_cookies,
        authoritative_cookies: options.authoritative_cookies,
        retained_cookies: options
            .authoritative_cookies
            .then_some(options.retained_cookies.as_slice()),
        proxy: ProxyRequest {
            url: format!("http://127.0.0.1:{}", proxy.port().unwrap()),
            username: "qbutt".into(),
            password: proxy.password().unwrap().into(),
        },
    };
    let mut input = serde_json::to_vec(&request).map_err(|_| WebViewError::InvalidRequest)?;
    if input.len() + 1 > MAX_REQUEST_BYTES {
        return Err(WebViewError::InvalidRequest);
    }
    input.push(b'\n');
    let mut command = Command::new(owner.app_dir.join("qbutt-webview.exe"));
    command.arg("--stdio");
    command.creation_flags(CREATE_NO_WINDOW);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command.kill_on_drop(true);
    let mut child = command.spawn().map_err(|_| WebViewError::Unavailable)?;
    let mut stdin = child.stdin.take().ok_or(WebViewError::Failed)?;
    let stdout = child.stdout.take().ok_or(WebViewError::Failed)?;
    let exchange = async {
        stdin
            .write_all(&input)
            .await
            .map_err(|_| WebViewError::Failed)?;
        let mut output = Vec::new();
        stdout
            .take(MAX_RESPONSE_BYTES + 1)
            .read_to_end(&mut output)
            .await
            .map_err(|_| WebViewError::Failed)?;
        if output.len() as u64 > MAX_RESPONSE_BYTES {
            return Err(WebViewError::Failed);
        }
        let status = child.wait().await.map_err(|_| WebViewError::Failed)?;
        if !status.success() && output.is_empty() {
            return Err(WebViewError::Failed);
        }
        let response: HelperResponse =
            serde_json::from_slice(&output).map_err(|_| WebViewError::Failed)?;
        if response.id != id {
            return Err(WebViewError::Failed);
        }
        if let Some(error) = response.error {
            return Err(match error.code.as_str() {
                "browser_check_required" => WebViewError::CheckRequired,
                "webview_runtime_required" => WebViewError::Unavailable,
                "webview_timeout" => WebViewError::Timeout,
                "canceled" => WebViewError::Canceled,
                _ => WebViewError::Failed,
            });
        }
        let session = response.session.ok_or(WebViewError::Failed)?;
        validate_session(&url, &session)?;
        Ok(session)
    };
    tokio::time::timeout(
        Duration::from_millis(u64::from(timeout_ms) + 3_000),
        exchange,
    )
    .await
    .map_err(|_| WebViewError::Timeout)?
}

fn validate_session(request: &Url, session: &WebViewSession) -> Result<(), WebViewError> {
    validate_media_url(&session.url).map_err(|_| WebViewError::Failed)?;
    if session.user_agent.is_empty()
        || session.user_agent.len() > 2048
        || session.user_agent.chars().any(char::is_control)
        || session.cookies.len() > 512
    {
        return Err(WebViewError::Failed);
    }
    let host = request.host_str().ok_or(WebViewError::Failed)?;
    for cookie in &session.cookies {
        let domain = cookie.domain.trim_start_matches('.').to_ascii_lowercase();
        if cookie.name.is_empty()
            || cookie.name.len() > 256
            || !cookie
                .name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
            || cookie.value.len() > 16 * 1024
            || cookie.value.chars().any(char::is_control)
            || cookie.path.len() > 2048
            || !cookie.path.starts_with('/')
            || cookie.path.chars().any(char::is_control)
            || !(domain == host || !cookie.host_only && host.ends_with(&format!(".{domain}")))
            || cookie
                .expires
                .is_some_and(|expiry| !expiry.is_finite() || expiry < 0.0)
        {
            return Err(WebViewError::Failed);
        }
    }
    Ok(())
}

fn digest(value: &[u8]) -> String {
    hex(&Sha256::digest(value))
}

fn hex(value: &[u8]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hex_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
