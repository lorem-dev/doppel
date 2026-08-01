//! The client half, used by `doppel config reload`.

use std::path::Path;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use super::{ControlRequest, ControlResponse};

/// Send a command and return the parsed response.
pub async fn send(socket: &Path, request: ControlRequest) -> anyhow::Result<ControlResponse> {
    let line = serde_json::to_string(&request)?;
    let raw = send_raw(socket, &line).await?;
    Ok(serde_json::from_str(raw.trim())?)
}

/// Send a raw line. Exists so tests can exercise malformed commands.
pub async fn send_raw(socket: &Path, line: &str) -> anyhow::Result<String> {
    let stream = UnixStream::connect(socket).await.map_err(|err| {
        anyhow::anyhow!(
            "cannot connect to the control socket at {} ({err}) -- is doppel running?",
            socket.display()
        )
    })?;

    let mut reader = BufReader::new(stream);
    reader
        .get_mut()
        .write_all(format!("{line}\n").as_bytes())
        .await?;
    reader.get_mut().flush().await?;

    let mut response = String::new();
    reader.read_line(&mut response).await?;
    Ok(response)
}
