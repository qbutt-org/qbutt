use std::collections::BTreeMap;
use std::sync::Arc;

use serde::Serialize;
use tokio::net::windows::named_pipe::NamedPipeServer;
use tokio::sync::{Notify, mpsc, oneshot, watch};
use tokio::task::JoinSet;
use tokio::time::{Instant, sleep_until};

use super::discovery::Discovery;
use super::transport::{self, Event};
use super::{
    BrowserChoice, BrowserCommand, BrowserConnection, BrowserError, BrowserId, BrowserReply,
    BrowserStatus, security,
};

pub(crate) struct Request {
    pub choice: BrowserChoice,
    pub command: BrowserCommand,
    pub reply: oneshot::Sender<Result<BrowserReply, BrowserError>>,
    pub deadline: Instant,
}

struct Client {
    metadata: Option<BrowserConnection>,
    outgoing: mpsc::Sender<Vec<u8>>,
    stop: watch::Sender<bool>,
}

struct Pending {
    connection: u64,
    command: BrowserCommand,
    reply: oneshot::Sender<Result<BrowserReply, BrowserError>>,
    deadline: Instant,
}

#[derive(Serialize)]
struct WireCommand<'a> {
    id: u64,
    #[serde(flatten)]
    command: &'a BrowserCommand,
}

pub(crate) async fn run(
    discovery: Discovery,
    mut listener: NamedPipeServer,
    mut incoming: mpsc::Receiver<Request>,
    status: watch::Sender<BrowserStatus>,
    mut stop: watch::Receiver<bool>,
    canceled: Arc<Notify>,
) {
    let (events, mut received) = mpsc::channel(32);
    let mut clients = BTreeMap::<u64, Client>::new();
    let mut pending = BTreeMap::<u64, Pending>::new();
    let mut tasks = JoinSet::new();
    let mut next_connection = 0u64;
    let mut next_request = 0u64;

    loop {
        if *stop.borrow() {
            break;
        }
        let deadline = pending.values().map(|request| request.deadline).min();
        let expiry = async {
            if let Some(deadline) = deadline {
                sleep_until(deadline).await;
            } else {
                std::future::pending::<()>().await;
            }
        };
        tokio::select! {
            _ = stop.changed() => break,
            result = listener.connect(), if tasks.len() < 4 => {
                if result.is_err() {
                    break;
                }
                let next = match security::create_pipe(discovery.pipe_name(), false) {
                    Ok(next) => next,
                    Err(_) => break,
                };
                let connected = std::mem::replace(&mut listener, next);
                next_connection += 1;
                let connection = next_connection;
                let (outgoing, output) = mpsc::channel(16);
                let (disconnect, stop_client) = watch::channel(false);
                clients.insert(connection, Client {
                    metadata: None,
                    outgoing,
                    stop: disconnect,
                });
                let events = events.clone();
                tasks.spawn(async move {
                    transport::connection(connected, connection, events, output, stop_client).await;
                    connection
                });
            }
            Some(event) = received.recv() => {
                match event {
                    Event::Hello { connection, metadata } => {
                        if !clients.contains_key(&connection) {
                            continue;
                        }
                        let replaced: Vec<_> = clients.iter().filter_map(|(id, client)| {
                            (*id != connection && client.metadata.as_ref()
                                .is_some_and(|old| old.browser == metadata.browser))
                                .then_some(*id)
                        }).collect();
                        for old in replaced {
                            disconnect(old, &mut clients, &mut pending);
                        }
                        clients.get_mut(&connection).expect("known connection").metadata = Some(metadata);
                        publish_status(&clients, &status);
                    }
                    Event::Response { connection, response } => {
                        // Replies from another connection cannot complete, fail or
                        // change the selected request, even if they guess its ID.
                        if pending.get(&response.id).is_none_or(|request| request.connection != connection) {
                            continue;
                        }
                        let request = pending.remove(&response.id).expect("known request");
                        let result = if let Some(error) = response.error {
                            Err(error.error())
                        } else {
                            request.command.reply(response.result.expect("validated response"))
                        };
                        let _ = request.reply.send(result);
                    }
                }
            }
            Some(completed) = tasks.join_next(), if !tasks.is_empty() => {
                match completed {
                    Ok(connection) => disconnect(connection, &mut clients, &mut pending),
                    Err(_) => break,
                }
                publish_status(&clients, &status);
            }
            Some(request) = incoming.recv() => {
                if request.reply.is_closed() {
                    continue;
                }
                if request.deadline <= Instant::now() {
                    let _ = request.reply.send(Err(BrowserError::Timeout));
                    continue;
                }
                let selected = choose(request.choice, &clients);
                let connection = match selected {
                    Ok(connection) if pending.len() < 16 => connection,
                    Ok(_) => {
                        let _ = request.reply.send(Err(BrowserError::Busy));
                        continue;
                    }
                    Err(error) => {
                        let _ = request.reply.send(Err(error));
                        continue;
                    }
                };
                let metadata = clients[&connection].metadata.as_ref().expect("selected client");
                if matches!(request.command, BrowserCommand::VkSearch { .. })
                    && !metadata.capabilities.iter().any(|value| value == "vk-search")
                {
                    let _ = request.reply.send(Err(BrowserError::UpdateRequired));
                    continue;
                }
                // Native messaging IDs must stay exactly representable in JS.
                if next_request >= 9_007_199_254_740_991 {
                    break;
                }
                next_request += 1;
                let id = next_request;
                let bytes = serde_json::to_vec(&WireCommand {
                    id,
                    command: &request.command,
                }).expect("serializable command");
                if clients[&connection].outgoing.try_send(bytes).is_err() {
                    let _ = request.reply.send(Err(BrowserError::Disconnected));
                    disconnect(connection, &mut clients, &mut pending);
                    publish_status(&clients, &status);
                    continue;
                }
                pending.insert(id, Pending {
                    connection,
                    deadline: request.deadline,
                    command: request.command,
                    reply: request.reply,
                });
            }
            _ = canceled.notified() => {},
            _ = expiry => {},
        }
        let expired: Vec<_> = pending
            .iter()
            .filter_map(|(id, request)| {
                (request.reply.is_closed() || request.deadline <= Instant::now()).then_some(*id)
            })
            .collect();
        for id in expired {
            // A full cancel queue disconnects its client and removes that
            // connection's remaining requests from this same expiry batch.
            let Some(request) = pending.remove(&id) else {
                continue;
            };
            let cancel = serde_json::to_vec(&serde_json::json!({ "id": id, "method": "cancel" }))
                .expect("constant command");
            if clients
                .get(&request.connection)
                .is_some_and(|client| client.outgoing.try_send(cancel).is_err())
            {
                disconnect(request.connection, &mut clients, &mut pending);
                publish_status(&clients, &status);
            }
            let _ = request.reply.send(Err(BrowserError::Timeout));
        }
    }

    incoming.close();
    for (_, request) in pending {
        let _ = request.reply.send(Err(BrowserError::Stopped));
    }
    while let Ok(request) = incoming.try_recv() {
        let _ = request.reply.send(Err(BrowserError::Stopped));
    }
    for (_, client) in clients {
        client.stop.send_replace(true);
    }
    // Connection loops select on stop, dropping both halves of their pipe and
    // any incomplete frame. Drain tasks before releasing the discovery owner.
    while tasks.join_next().await.is_some() {}
    drop(listener);
    drop(discovery);
    status.send_replace(BrowserStatus {
        connections: Vec::new(),
        stopped: true,
    });
}

fn choose(choice: BrowserChoice, clients: &BTreeMap<u64, Client>) -> Result<u64, BrowserError> {
    let connected: Vec<_> = clients
        .iter()
        .filter_map(|(id, client)| {
            client
                .metadata
                .as_ref()
                .map(|metadata| (*id, metadata.browser))
        })
        .collect();
    match choice {
        BrowserChoice::Auto => match connected.as_slice() {
            [(id, _)] => Ok(*id),
            [] => Err(BrowserError::BrowserRequired),
            _ => Err(BrowserError::ChoiceRequired),
        },
        BrowserChoice::Edge | BrowserChoice::Chrome => {
            let browser = if choice == BrowserChoice::Edge {
                BrowserId::Edge
            } else {
                BrowserId::Chrome
            };
            connected
                .into_iter()
                .find_map(|(id, candidate)| (candidate == browser).then_some(id))
                .ok_or(BrowserError::BrowserRequired)
        }
    }
}

fn disconnect(
    connection: u64,
    clients: &mut BTreeMap<u64, Client>,
    pending: &mut BTreeMap<u64, Pending>,
) {
    if let Some(client) = clients.remove(&connection) {
        client.stop.send_replace(true);
    }
    let affected: Vec<_> = pending
        .iter()
        .filter_map(|(id, request)| (request.connection == connection).then_some(*id))
        .collect();
    for id in affected {
        let request = pending.remove(&id).expect("affected request");
        let _ = request.reply.send(Err(BrowserError::Disconnected));
    }
}

fn publish_status(clients: &BTreeMap<u64, Client>, status: &watch::Sender<BrowserStatus>) {
    status.send_replace(BrowserStatus {
        connections: clients
            .values()
            .filter_map(|client| client.metadata.clone())
            .collect(),
        stopped: false,
    });
}
