//! IPC client — used by bridge scripts to connect to the daemon.

use crate::protocol::{IpcMessage, IpcResponse};
use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::debug;

/// Send a message to the AgentToast daemon and wait for a response.
pub async fn send_to_daemon(
    pipe_name: &str,
    auth_token: &str,
    message: &IpcMessage,
) -> Result<IpcResponse> {
    #[cfg(windows)]
    let stream = connect_windows(pipe_name).await?;
    
    #[cfg(unix)]
    let stream = connect_unix(pipe_name).await?;

    exchange(stream, auth_token, message).await
}

#[cfg(windows)]
async fn connect_windows(pipe_name: &str) -> Result<tokio::net::windows::named_pipe::NamedPipeClient> {
    use tokio::net::windows::named_pipe::ClientOptions;

    let client = ClientOptions::new()
        .open(pipe_name)
        .with_context(|| {
            format!(
                "Failed to connect to AgentToast daemon at {}. Is the daemon running?",
                pipe_name
            )
        })?;

    Ok(client)
}

#[cfg(unix)]
async fn connect_unix(socket_path: &str) -> Result<tokio::net::UnixStream> {
    tokio::net::UnixStream::connect(socket_path)
        .await
        .with_context(|| {
            format!(
                "Failed to connect to AgentToast daemon at {}. Is the daemon running?",
                socket_path
            )
        })
}

/// Exchange a message with the daemon over an established connection.
async fn exchange<S>(
    stream: S,
    auth_token: &str,
    message: &IpcMessage,
) -> Result<IpcResponse>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (reader, mut writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(reader);

    // Step 1: Send auth token
    writer.write_all(auth_token.as_bytes()).await?;
    writer.write_all(b"\n").await?;

    // Step 2: Send message
    let message_json = serde_json::to_string(message)?;
    debug!(message = %message_json, "Sending IPC message");
    writer.write_all(message_json.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;

    // Step 3: Read response
    let mut response_line = String::new();
    reader.read_line(&mut response_line).await
        .context("Failed to read response from daemon")?;

    let response: IpcResponse = serde_json::from_str(response_line.trim())
        .with_context(|| format!("Failed to parse daemon response: {}", response_line.trim()))?;

    Ok(response)
}
