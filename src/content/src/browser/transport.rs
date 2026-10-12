use std::io;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::windows::named_pipe::NamedPipeServer;
use tokio::sync::{mpsc, watch};
use tokio::time::timeout;

use super::BrowserConnection;
use super::protocol::{Hello, MAX_FRAME_BYTES, Response};

pub(crate) enum Event {
    Hello {
        connection: u64,
        metadata: BrowserConnection,
    },
    Response {
        connection: u64,
        response: Response,
    },
}

/// Each direction retains its read/write future until completion. Recreating a
/// read_exact future after every outgoing frame would lose a partial header.
pub(crate) async fn connection(
    pipe: NamedPipeServer,
    id: u64,
    events: mpsc::Sender<Event>,
    mut outgoing: mpsc::Receiver<Vec<u8>>,
    mut stop: watch::Receiver<bool>,
) {
    let (mut reader, mut writer) = tokio::io::split(pipe);
    let hello = async {
        let bytes = read_frame(&mut reader).await?;
        let hello: Hello = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        let metadata = hello.connection().map_err(|_| invalid())?;
        events
            .send(Event::Hello {
                connection: id,
                metadata,
            })
            .await
            .map_err(|_| invalid())
    };
    tokio::select! {
        result = timeout(Duration::from_secs(5), hello) => {
            if !matches!(result, Ok(Ok(()))) { return; }
        }
        _ = stop.changed() => return,
    }
    let reading = read_responses(&mut reader, id, &events);
    let writing = async {
        while let Some(bytes) = outgoing.recv().await {
            timeout(Duration::from_secs(5), write_frame(&mut writer, &bytes))
                .await
                .map_err(|_| invalid())??;
        }
        Ok::<(), io::Error>(())
    };
    tokio::select! {
        _ = reading => {},
        _ = writing => {},
        _ = stop.changed() => {},
    }
}

async fn read_responses(
    reader: &mut (impl AsyncRead + Unpin),
    connection: u64,
    events: &mpsc::Sender<Event>,
) -> io::Result<()> {
    loop {
        let bytes = read_frame(reader).await?;
        let response: Response = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if response.id == 0
            || response.id > 9_007_199_254_740_991
            || response.result.is_some() == response.error.is_some()
        {
            return Err(invalid());
        }
        events
            .send(Event::Response {
                connection,
                response,
            })
            .await
            .map_err(|_| invalid())?;
    }
}

async fn read_frame(reader: &mut (impl AsyncRead + Unpin)) -> io::Result<Vec<u8>> {
    // Idle connections may remain open, but once a header starts its complete
    // frame has a deadline. The first byte is kept outside that deadline.
    let mut header = [0u8; 4];
    reader.read_exact(&mut header[..1]).await?;
    timeout(Duration::from_secs(5), async {
        reader.read_exact(&mut header[1..]).await?;
        let length = u32::from_le_bytes(header) as usize;
        if length == 0 || length > MAX_FRAME_BYTES {
            return Err(invalid());
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).await?;
        Ok(body)
    })
    .await
    .map_err(|_| invalid())?
}

async fn write_frame(writer: &mut (impl AsyncWrite + Unpin), bytes: &[u8]) -> io::Result<()> {
    if bytes.is_empty() || bytes.len() > MAX_FRAME_BYTES {
        return Err(invalid());
    }
    writer
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .await?;
    writer.write_all(bytes).await?;
    writer.flush().await
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Invalid browser frame")
}
