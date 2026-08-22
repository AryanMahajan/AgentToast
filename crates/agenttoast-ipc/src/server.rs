//! IPC server — listens for connections from bridge scripts.
//!
//! On Windows, uses named pipes.
//! On Unix, uses Unix domain sockets.

use crate::auth;
use crate::protocol::{IpcMessage, IpcResponse};
use agenttoast_core::config::AppConfig;
use anyhow::{Context, Result};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

/// Event emitted by the IPC server when a bridge sends a message.
#[derive(Debug)]
pub struct IncomingEvent {
    pub message: IpcMessage,
    /// Channel to send the response back to the bridge
    pub response_tx: tokio::sync::oneshot::Sender<IpcResponse>,
}

/// Start the IPC server.
///
/// Returns a receiver that yields incoming events from bridge scripts.
pub async fn start_server(
    config: &AppConfig,
    auth_token: &str,
) -> Result<mpsc::Receiver<IncomingEvent>> {
    let (event_tx, event_rx) = mpsc::channel::<IncomingEvent>(64);
    let pipe_name = config.ipc.pipe_name.clone();
    let token = auth_token.to_string();

    info!(pipe = %pipe_name, "Starting IPC server");

    tokio::spawn(async move {
        if let Err(e) = run_pipe_server(&pipe_name, &token, event_tx).await {
            error!(error = %e, "IPC server error");
        }
    });

    Ok(event_rx)
}

/// Windows named pipe server implementation.
#[cfg(windows)]
async fn run_pipe_server(
    pipe_name: &str,
    auth_token: &str,
    event_tx: mpsc::Sender<IncomingEvent>,
) -> Result<()> {
    use tokio::net::windows::named_pipe::ServerOptions;

    loop {
        let server = ServerOptions::new()
            .first_pipe_instance(false)
            .create(pipe_name)
            .with_context(|| format!("Failed to create named pipe: {}", pipe_name))?;

        info!("Waiting for IPC connection...");
        server.connect().await?;
        info!("IPC client connected");

        let token = auth_token.to_string();
        let tx = event_tx.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_connection(server, &token, tx).await {
                error!(error = %e, "Connection handler error");
            }
        });
    }
}

/// Unix socket server implementation.
#[cfg(unix)]
async fn run_pipe_server(
    socket_path: &str,
    auth_token: &str,
    event_tx: mpsc::Sender<IncomingEvent>,
) -> Result<()> {
    use tokio::net::UnixListener;

    // Remove stale socket file
    let _ = std::fs::remove_file(socket_path);

    let listener = UnixListener::bind(socket_path)
        .with_context(|| format!("Failed to bind Unix socket: {}", socket_path))?;

    loop {
        let (stream, _) = listener.accept().await?;
        let token = auth_token.to_string();
        let tx = event_tx.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, &token, tx).await {
                error!(error = %e, "Connection handler error");
            }
        });
    }
}

/// Handle a single IPC connection.
///
/// Protocol:
/// 1. Client sends auth token as first line
/// 2. Client sends JSON message as second line
/// 3. Server sends JSON response
async fn handle_connection<S>(
    stream: S,
    auth_token: &str,
    event_tx: mpsc::Sender<IncomingEvent>,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (reader, mut writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(reader);

    // Step 1: Read and verify auth token
    let mut token_line = String::new();
    reader.read_line(&mut token_line).await?;
    let received_token = token_line.trim();

    if received_token != auth_token {
        warn!("Authentication failed: invalid token");
        let response = serde_json::to_string(&IpcResponse::error("Authentication failed"))?;
        writer.write_all(response.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        return Ok(());
    }

    // Step 2: Read the message
    let mut message_line = String::new();
    reader.read_line(&mut message_line).await?;

    let message: IpcMessage = serde_json::from_str(message_line.trim())
        .with_context(|| format!("Failed to parse IPC message: {}", message_line.trim()))?;

    // Step 3: Forward to the daemon and wait for response
    let (response_tx, response_rx) = tokio::sync::oneshot::channel();
    let event = IncomingEvent {
        message,
        response_tx,
    };

    event_tx.send(event).await
        .map_err(|_| anyhow::anyhow!("Event channel closed"))?;

    // Wait for the daemon to process and respond
    let response = response_rx.await
        .unwrap_or_else(|_| IpcResponse::error("Internal error: response channel dropped"));

    let response_json = serde_json::to_string(&response)?;
    writer.write_all(response_json.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;

    Ok(())
}
