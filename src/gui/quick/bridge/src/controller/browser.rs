use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::RwLock;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QByteArray;
use qbutt_content::browser::{
    BrowserBroker, BrowserChoice, BrowserCommand, BrowserError, BrowserHandle, BrowserId,
    BrowserStatus,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Runtime;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use super::ffi;

const MAX_REQUESTS: usize = 16;
const MAX_MESSAGE_BYTES: usize = 768 * 1024;
const MAX_BROWSER_ID: u64 = 9_007_199_254_740_991;

pub(super) struct BrowserService {
    client: BrowserClient,
    initialization: Option<JoinHandle<Result<(), BrowserError>>>,
}

#[derive(Clone)]
pub(super) struct BrowserClient {
    state: Arc<Mutex<BrowserState>>,
    choice: Arc<RwLock<BrowserChoice>>,
}

pub(super) fn choice(value: &str) -> BrowserChoice {
    match value {
        "edge" => BrowserChoice::Edge,
        "chrome" => BrowserChoice::Chrome,
        _ => BrowserChoice::Auto,
    }
}

enum BrowserState {
    Idle(PathBuf),
    Running(BrowserBroker),
    Stopped,
}

impl BrowserClient {
    pub(super) fn choice(&self) -> BrowserChoice {
        *self.choice.read().expect("browser choice lock")
    }

    pub(super) fn set_choice(&self, value: BrowserChoice) {
        *self.choice.write().expect("browser choice lock") = value;
    }

    pub(super) async fn account_choice(&self, choice: BrowserChoice) -> Option<BrowserChoice> {
        if choice != BrowserChoice::Auto {
            return Some(choice);
        }

        let Some(handle) = self.existing_handle().await else {
            return Some(BrowserChoice::Auto);
        };
        match handle.status().connections.as_slice() {
            [] => Some(BrowserChoice::Auto),
            [connection] => Some(match connection.browser {
                BrowserId::Edge => BrowserChoice::Edge,
                BrowserId::Chrome => BrowserChoice::Chrome,
            }),
            _ => None,
        }
    }

    pub(super) async fn handle(&self) -> Result<BrowserHandle, BrowserError> {
        let mut state = self.state.lock().await;
        match &*state {
            BrowserState::Idle(discovery) => {
                let broker = BrowserBroker::start(discovery.clone()).await?;
                let handle = broker.handle();
                *state = BrowserState::Running(broker);
                Ok(handle)
            }
            BrowserState::Running(broker) => Ok(broker.handle()),
            BrowserState::Stopped => Err(BrowserError::Stopped),
        }
    }

    pub(super) async fn existing_handle(&self) -> Option<BrowserHandle> {
        match &*self.state.lock().await {
            BrowserState::Running(broker) => Some(broker.handle()),
            BrowserState::Idle(_) | BrowserState::Stopped => None,
        }
    }
}

impl BrowserService {
    pub(super) fn new(discovery: PathBuf) -> Self {
        Self {
            client: BrowserClient {
                state: Arc::new(Mutex::new(BrowserState::Idle(discovery))),
                choice: Arc::new(RwLock::new(BrowserChoice::Auto)),
            },
            initialization: None,
        }
    }

    pub(super) fn client(&self) -> BrowserClient {
        self.client.clone()
    }

    fn initialize(&mut self, runtime: &Runtime) {
        if self.initialization.is_some() {
            return;
        }
        let client = self.client();
        self.initialization = Some(runtime.spawn(async move { client.handle().await.map(|_| ()) }));
    }

    pub(super) async fn shutdown(&mut self) {
        let state = std::mem::replace(&mut *self.client.state.lock().await, BrowserState::Stopped);
        if let Some(initialization) = self.initialization.take() {
            let _ = initialization.await;
        }
        if let BrowserState::Running(broker) = state {
            let _ = broker.shutdown().await;
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RelayRequest {
    id: u64,
    browser: BrowserChoice,
    command: RelayCommand,
    #[serde(default)]
    optional: bool,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RelayCommand {
    Control(ControlCommand),
    Browser(Box<BrowserCommand>),
}

#[derive(Deserialize)]
#[serde(tag = "method", rename_all = "lowercase", deny_unknown_fields)]
enum ControlCommand {
    Status,
    Initialize,
}

fn failure(error: BrowserError) -> Value {
    json!({ "error": { "code": error.code(), "message": error.to_string() } })
}

fn encode_response(response: Value) -> Vec<u8> {
    let encoded = serde_json::to_vec(&response).expect("JSON value serializes");
    if encoded.len() <= MAX_MESSAGE_BYTES {
        encoded
    } else {
        serde_json::to_vec(&failure(BrowserError::InvalidResponse))
            .expect("static failure serializes")
    }
}

fn native_host_missing(
    status: &BrowserStatus,
    selected: BrowserChoice,
    installed: &[ffi::BrowserInstallation],
) -> bool {
    let browser = match selected {
        BrowserChoice::Auto => None,
        BrowserChoice::Edge => Some("edge"),
        BrowserChoice::Chrome => Some("chrome"),
    };
    if status
        .connections
        .iter()
        .any(|peer| browser.is_none_or(|browser| peer.browser.as_str() == browser))
    {
        return false;
    }
    installed
        .iter()
        .filter(|entry| browser.is_none_or(|browser| entry.id.to_string() == browser))
        .all(|entry| entry.native_host == ffi::NativeHostState::Missing)
}

fn recovery_page(command: &BrowserCommand) -> String {
    let mut page = match command {
        BrowserCommand::Resolve { url, referrer, .. } => referrer.as_ref().unwrap_or(url).clone(),
        BrowserCommand::Cookies { url }
        | BrowserCommand::LibSession { url }
        | BrowserCommand::Fetch { url, .. }
        | BrowserCommand::VkSearch { url, .. } => url.clone(),
    };
    page.set_query(None);
    page.set_fragment(None);
    page.to_string()
}

fn browser_failure(error: BrowserError, page: &str) -> Value {
    let mut result = failure(error);
    if matches!(
        error,
        BrowserError::BrowserCheckRequired | BrowserError::VerificationRequired
    ) {
        result["error"]["url"] = json!(page);
    }
    result
}

fn status(
    status: BrowserStatus,
    selected: BrowserChoice,
    installed: Vec<ffi::BrowserInstallation>,
) -> Value {
    let connected = status.connections;
    let chosen = match selected {
        BrowserChoice::Auto if connected.len() == 1 => connected.first(),
        BrowserChoice::Auto => None,
        BrowserChoice::Edge => connected
            .iter()
            .find(|peer| peer.browser.as_str() == "edge"),
        BrowserChoice::Chrome => connected
            .iter()
            .find(|peer| peer.browser.as_str() == "chrome"),
    };
    let detected = installed
        .iter()
        .find(|browser| browser.is_default)
        .map(|browser| browser.id.to_string());
    let browsers: Vec<_> = installed
        .into_iter()
        .filter(|browser| browser.installed)
        .map(|browser| {
            let id = browser.id.to_string();
            json!({
                "connected": connected.iter().any(|peer| peer.browser.as_str() == id),
                "id": id,
                "name": browser.name.to_string(),
                "isDefault": browser.is_default,
            })
        })
        .collect();
    let mut result =
        json!({ "selected": selected, "ready": chosen.is_some(), "browsers": browsers });
    if let Some(detected) = detected {
        result["detected"] = json!(detected);
    }
    if let Some(chosen) = chosen {
        result["userAgent"] = json!(chosen.user_agent);
        result["connectedBrowser"] = json!(chosen.browser);
    }
    result
}

impl ffi::ContentController {
    pub fn initialize_browser(mut self: Pin<&mut Self>) {
        if let Some(services) = self.as_mut().rust_mut().services.as_mut() {
            services.browser.initialize(&services.runtime);
        }
    }

    pub fn relay_browser_request(
        mut self: Pin<&mut Self>,
        generation: u64,
        id: u64,
        request: QByteArray,
    ) {
        let key = (generation, id);
        if self.browser_requests.contains_key(&key) {
            return;
        }
        let parsed = (request.as_slice().len() <= MAX_MESSAGE_BYTES)
            .then(|| serde_json::from_slice::<RelayRequest>(request.as_slice()).ok())
            .flatten()
            // Lib account credentials remain inside the native provider path.
            .filter(|request| {
                !matches!(&request.command, RelayCommand::Browser(command)
                if matches!(command.as_ref(), BrowserCommand::LibSession { .. }))
            })
            .filter(|request| request.id == id && id > 0 && id <= MAX_BROWSER_ID);
        let Some(request) = parsed else {
            let reply = encode_response(failure(BrowserError::InvalidRequest));
            self.browser_reply(generation, id, QByteArray::from(reply.as_slice()));
            return;
        };
        if request.optional
            && !matches!(&request.command, RelayCommand::Browser(command)
            if matches!(command.as_ref(), BrowserCommand::Cookies { .. }))
        {
            let reply = encode_response(failure(BrowserError::InvalidRequest));
            self.browser_reply(generation, id, QByteArray::from(reply.as_slice()));
            return;
        }
        if self.browser_requests.len() >= MAX_REQUESTS || self.services.is_none() {
            let reply = encode_response(failure(BrowserError::Busy));
            self.browser_reply(generation, id, QByteArray::from(reply.as_slice()));
            return;
        }
        let services = self.services.as_ref().unwrap();
        let client = services.browser.client();
        let qt_thread = self.qt_thread();
        let task = services.runtime.spawn(async move {
            let installed = ffi::browser_installations();
            let response = if matches!(
                request.command,
                RelayCommand::Control(ControlCommand::Status)
            ) {
                let current = client
                    .existing_handle()
                    .await
                    .as_ref()
                    .map(BrowserHandle::status)
                    .unwrap_or_default();
                json!({ "result": status(current, request.browser, installed) })
            } else {
                let handle = if request.optional {
                    Ok(client.existing_handle().await)
                } else {
                    client.handle().await.map(Some)
                };
                match handle {
                    Ok(None) => json!({ "result": [] }),
                    Ok(Some(handle)) => match request.command {
                        RelayCommand::Control(_) => {
                            json!({ "result": status(handle.status(), request.browser, installed) })
                        }
                        RelayCommand::Browser(command) => {
                            if let Err(error) = command.validate() {
                                failure(error)
                            } else if request.optional {
                                match handle
                                    .request_connected(request.browser, *command)
                                    .await
                                    .and_then(|reply| reply.into_wire())
                                {
                                    Ok(reply) => json!({ "result": reply }),
                                    Err(
                                        BrowserError::BrowserRequired
                                        | BrowserError::ChoiceRequired
                                        | BrowserError::Disconnected
                                        | BrowserError::Stopped,
                                    ) => json!({ "result": [] }),
                                    Err(error) => failure(error),
                                }
                            } else if native_host_missing(
                                &handle.status(),
                                request.browser,
                                &installed,
                            ) {
                                failure(BrowserError::BrowserRequired)
                            } else {
                                let page = recovery_page(&command);
                                match handle
                                    .request(request.browser, *command)
                                    .await
                                    .and_then(|reply| reply.into_wire())
                                {
                                    Ok(reply) => json!({ "result": reply }),
                                    Err(error) => browser_failure(error, &page),
                                }
                            }
                        }
                    },
                    Err(error) => failure(error),
                }
            };
            // Only bounded, validated qcatch payloads cross back to the private helper.
            let encoded = encode_response(response);
            let _ = qt_thread.queue(move |mut controller| {
                if controller
                    .as_mut()
                    .rust_mut()
                    .browser_requests
                    .remove(&key)
                    .is_some()
                {
                    controller.browser_reply(generation, id, QByteArray::from(encoded.as_slice()));
                }
            });
        });
        self.as_mut().rust_mut().browser_requests.insert(key, task);
    }

    pub fn cancel_browser_request(mut self: Pin<&mut Self>, generation: u64, id: u64) {
        if let Some(task) = self
            .as_mut()
            .rust_mut()
            .browser_requests
            .remove(&(generation, id))
        {
            task.abort();
        }
    }

    pub fn close_browser_session(mut self: Pin<&mut Self>, generation: u64) {
        let requests = &mut self.as_mut().rust_mut().browser_requests;
        requests.retain(|(session, _), task| {
            if *session == generation {
                task.abort();
                false
            } else {
                true
            }
        });
    }

    pub fn close_browser_relay(mut self: Pin<&mut Self>) {
        let reply = encode_response(failure(BrowserError::Stopped));
        let requests = std::mem::take(&mut self.as_mut().rust_mut().browser_requests);
        for ((generation, id), task) in requests {
            task.abort();
            self.as_mut()
                .browser_reply(generation, id, QByteArray::from(reply.as_slice()));
        }
    }
}
