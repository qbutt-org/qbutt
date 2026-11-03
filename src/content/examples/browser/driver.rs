//! Private JSONL transport for generated browser integration. The caller keeps
//! stdout replies ephemeral and logs only redacted assertions, never wire data.

use std::path::PathBuf;

use qbutt_content::browser::{BrowserBroker, BrowserChoice, BrowserCommand, BrowserError};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, BufWriter, Stdout};

const MAX_INPUT_BYTES: u64 = 32 * 1024;
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
type Result = std::result::Result<(), Box<dyn std::error::Error>>;

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "lowercase", deny_unknown_fields)]
enum Input {
    Status {
        id: u64,
    },
    Request {
        id: u64,
        choice: BrowserChoice,
        command: Box<BrowserCommand>,
    },
}

pub(super) async fn run(discovery: PathBuf) -> Result {
    let broker = BrowserBroker::start(discovery).await?;
    let handle = broker.handle();
    let outcome: Result = async {
        let mut input = BufReader::new(tokio::io::stdin());
        let mut output = BufWriter::new(tokio::io::stdout());
        write_response(&mut output, json!({ "ready": true })).await?;
        loop {
            let mut line = Vec::new();
            let count = (&mut input)
                .take(MAX_INPUT_BYTES + 1)
                .read_until(b'\n', &mut line)
                .await?;
            if count == 0 {
                break;
            }
            if count as u64 > MAX_INPUT_BYTES {
                return Err("Browser fixture input exceeds its limit".into());
            }
            let request: Input = serde_json::from_slice(&line)
                .map_err(|_| "Invalid typed browser fixture request")?;
            let response = match request {
                Input::Status { id } => {
                    let status = handle.status();
                    json!({ "id": id, "result": {
                        "connections": status.connections, "stopped": status.stopped,
                    } })
                }
                Input::Request {
                    id,
                    choice,
                    command,
                } => {
                    let result =
                        handle
                            .request_connected(choice, *command)
                            .await
                            .and_then(|reply| {
                                reply.into_wire().map_err(|_| BrowserError::InvalidResponse)
                            });
                    match result {
                        Ok(reply) => json!({ "id": id, "result": reply }),
                        Err(error) => json!({ "id": id, "error": error.code() }),
                    }
                }
            };
            write_response(&mut output, response).await?;
        }
        Ok(())
    }
    .await;
    let stopped = broker.shutdown().await;
    outcome?;
    stopped?;
    Ok(())
}

async fn write_response(output: &mut BufWriter<Stdout>, value: Value) -> Result {
    let mut bytes =
        serde_json::to_vec(&value).map_err(|_| "Could not encode the browser fixture reply")?;
    if bytes.len() > MAX_OUTPUT_BYTES {
        return Err("Browser fixture output exceeds its limit".into());
    }
    bytes.push(b'\n');
    output.write_all(&bytes).await?;
    output.flush().await?;
    Ok(())
}
