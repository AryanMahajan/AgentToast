//! AgentToast Bridge for Claude Code
//!
//! This binary is invoked by Claude Code's hook system.
//! It reads the hook payload from stdin, sends it to the AgentToast daemon,
//! waits for the user's response, and writes the response to stdout.

use agenttoast_adapters::claude::{ClaudeAdapter, HookKind};
use agenttoast_adapters::AgentAdapter;
use agenttoast_core::config::AppConfig;
use agenttoast_ipc::auth;
use agenttoast_ipc::protocol::{IpcMessage, IpcResponse};
use anyhow::{Context, Result};
use std::io::{self, Read};
use std::time::Duration;
use tracing::{debug, error, info, warn};

/// Registry bookkeeping is not worth stalling a session over.
const NOTIFY_TIMEOUT: Duration = Duration::from_secs(5);

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
        error!(error = ?e, "Bridge failed");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let config = AppConfig::load();
    let adapter = ClaudeAdapter;

    let mut stdin_data = String::new();
    io::stdin()
        .read_to_string(&mut stdin_data)
        .context("Failed to read from stdin")?;

    debug!(payload = %stdin_data, "Received hook payload");

    // Some shells hand over a UTF-8 BOM; serde_json rejects it outright.
    let payload: serde_json::Value = serde_json::from_str(stdin_data.trim_start_matches('\u{feff}'))
        .context("Failed to parse stdin as JSON")?;

    let auth_token = auth::read_token(&config.data_dir)
        .context("Failed to read auth token. Is the AgentToast daemon running?")?;

    // Session lifecycle hooks only maintain the registry; they have no decision
    // to wait for, so they report and exit rather than blocking the agent.
    match HookKind::from_payload(&payload) {
        HookKind::SessionStart => {
            return register_session(&config, &auth_token, &payload).await;
        }
        HookKind::SessionEnd => {
            return deregister_session(&config, &auth_token, &payload).await;
        }
        HookKind::Attention => {}
    }

    let event = adapter
        .parse_hook_payload(&payload)
        .context("Failed to parse hook payload")?;

    let mut event = event;
    event.process_id = agent_pid();

    info!(
        session_id = %event.session_id,
        tool = ?event.tool_name,
        pid = ?event.process_id,
        message = %event.message,
        "Attention event created"
    );

    let ipc_message = IpcMessage::Attention {
        event: event.clone(),
    };

    // Bound the wait. Writing nothing to stdout and exiting 0 makes Claude Code
    // fall back to its own permission prompt, so a user who never answers the
    // toast is left deciding in the session rather than stuck forever.
    let response = match tokio::time::timeout(
        config.bridge_timeout,
        agenttoast_ipc::client::send_to_daemon(&config.ipc.pipe_name, &auth_token, &ipc_message),
    )
    .await
    {
        Ok(result) => result.context("Failed to communicate with AgentToast daemon")?,
        Err(_) => {
            warn!(
                timeout_secs = config.bridge_timeout.as_secs(),
                "No response from AgentToast in time; deferring to the session"
            );
            return Ok(());
        }
    };

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

/// Announce a new session so the registry knows about it before any tool runs.
async fn register_session(
    config: &AppConfig,
    auth_token: &str,
    payload: &serde_json::Value,
) -> Result<()> {
    let session_id = ClaudeAdapter.extract_session_id(payload)?;
    let working_directory = payload
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    info!(session_id = %session_id, cwd = ?working_directory, "Registering session");

    notify(
        config,
        auth_token,
        &IpcMessage::Register {
            agent: "claude".to_string(),
            session_id,
            process_id: agent_pid(),
            working_directory,
        },
    )
    .await
}

/// Drop a finished session from the registry.
async fn deregister_session(
    config: &AppConfig,
    auth_token: &str,
    payload: &serde_json::Value,
) -> Result<()> {
    let session_id = ClaudeAdapter.extract_session_id(payload)?;
    info!(session_id = %session_id, "Deregistering session");

    notify(
        config,
        auth_token,
        &IpcMessage::Deregister { session_id },
    )
    .await
}

/// Send a fire-and-forget bookkeeping message.
///
/// Registry upkeep must never hold up or fail a session: if the daemon is not
/// running, log it and let the hook succeed.
async fn notify(config: &AppConfig, auth_token: &str, message: &IpcMessage) -> Result<()> {
    let send = agenttoast_ipc::client::send_to_daemon(&config.ipc.pipe_name, auth_token, message);

    match tokio::time::timeout(NOTIFY_TIMEOUT, send).await {
        Ok(Ok(_)) => debug!("Registry updated"),
        Ok(Err(e)) => warn!(error = %e, "Could not reach AgentToast daemon"),
        Err(_) => warn!("Timed out updating the AgentToast registry"),
    }
    Ok(())
}

/// PID of the process that spawned this bridge — i.e. the agent itself.
///
/// The bridge is a short-lived child of Claude Code, so its parent is the
/// process the daemon needs in order to find and focus the session's window.
fn agent_pid() -> Option<u32> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

    let me = Pid::from_u32(std::process::id());
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[me]),
        true,
        ProcessRefreshKind::nothing(),
    );

    system.process(me).and_then(|p| p.parent()).map(|p| p.as_u32())
}
