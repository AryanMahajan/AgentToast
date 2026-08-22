//! AgentToast Bridge for Claude Code

use agenttoast_adapters::claude::ClaudeAdapter;
use agenttoast_adapters::AgentAdapter;
use agenttoast_core::config::AppConfig;
use agenttoast_ipc::auth;
use agenttoast_ipc::protocol::{IpcMessage, IpcResponse};
use anyhow::{Context, Result};
use std::io::{self, Read};
use tracing::{debug, error, info};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("agenttoast=debug".parse().unwrap()),
        )
        .init();

    if let Err(e) = run().await {
        error!(error = %e, "Bridge failed");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let config = AppConfig::default();
    let adapter = ClaudeAdapter;

    let mut stdin_data = String::new();
    io::stdin()
        .read_to_string(&mut stdin_data)
        .context("Failed to read from stdin")?;

    debug!(payload = %stdin_data, "Received hook payload");

    let payload: serde_json::Value = serde_json::from_str(&stdin_data)
        .context("Failed to parse stdin as JSON")?;

    let event = adapter
        .parse_hook_payload(&payload)
        .context("Failed to parse hook payload")?;

    info!(
        session_id = %event.session_id,
        tool = ?event.tool_name,
        message = %event.message,
        "Attention event created"
    );

    let auth_token = auth::read_token(&config.data_dir)
        .context("Failed to read auth token. Is the AgentToast daemon running?")?;

    let ipc_message = IpcMessage::Attention {
        event: event.clone(),
    };

    #[cfg(windows)]
    let response = agenttoast_ipc::client::send_to_daemon_windows(
        &config.ipc.pipe_name,
        &auth_token,
        &ipc_message,
    )
    .await
    .context("Failed to communicate with AgentToast daemon")?;

    #[cfg(unix)]
    let response = agenttoast_ipc::client::send_to_daemon(
        &config.ipc.pipe_name,
        &auth_token,
        &ipc_message,
    )
    .await
    .context("Failed to communicate with AgentToast daemon")?;

    match response {
        IpcResponse::UserAction {
            action,
            text_input,
            ..
        } => {
            let hook_response = adapter.format_response(action, text_input.as_deref())?;
            info!(response = %hook_response, "Sending response to Claude Code");
            print!("{}", hook_response);
        }
        IpcResponse::Error { message } => {
            error!(error = %message, "Daemon returned error");
        }
        IpcResponse::Ack { .. } => {
            debug!("Received ack instead of user action");
        }
    }

    Ok(())
}
