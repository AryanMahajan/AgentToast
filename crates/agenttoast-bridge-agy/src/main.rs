//! AgentToast Bridge for Antigravity (`agy`)
//!
//! Antigravity runs this on every tool call that matches the hook's matcher.
//! It reads the hook payload from stdin, hands it to the AgentToast daemon,
//! waits for the user's answer, and writes the decision to stdout.
//!
//! The one rule that shapes everything here: **a failing hook takes Antigravity
//! down with it.** A non-zero exit, or a decision it cannot parse, does not
//! degrade to "carry on without the hook" — it fails the tool call, and with a
//! matcher on `run_command` that means the agent cannot do anything at all.
//! So every failure path in this binary ends the same way: write nothing, exit
//! zero, and let Antigravity fall back to its own permission prompt.

use agenttoast_adapters::agy::{AgyAdapter, AgyHookKind};
use agenttoast_adapters::AgentAdapter;
use agenttoast_core::config::AppConfig;
use agenttoast_ipc::auth;
use agenttoast_ipc::protocol::{IpcMessage, IpcResponse};
use anyhow::{Context, Result};
use std::io::{self, Read};
use std::time::Duration;
use tracing::{debug, error, info, warn};

/// Nothing is waiting on a "finished" toast, so it is not worth stalling the
/// end of a turn over.
const NOTIFY_TIMEOUT: Duration = Duration::from_secs(5);

#[tokio::main]
async fn main() {
    // Quieter by default than the Claude Code bridge: Antigravity captures a
    // hook's stderr and reports it, so per-call debug output would end up in
    // front of the model on every tool call. Set RUST_LOG to get it back.
    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    if let Err(e) = run().await {
        // Deliberately not `exit(1)`. See the note at the top of the file:
        // a failed hook blocks every tool call, so a broken bridge must look
        // like an absent one.
        error!(error = ?e, "Bridge failed; deferring to Antigravity's own prompt");
    }
}

async fn run() -> Result<()> {
    let config = AppConfig::load();
    let adapter = AgyAdapter;

    let mut stdin_data = String::new();
    io::stdin()
        .read_to_string(&mut stdin_data)
        .context("Failed to read from stdin")?;

    debug!(payload = %stdin_data, "Received hook payload");

    let payload: serde_json::Value = serde_json::from_str(stdin_data.trim_start_matches('\u{feff}'))
        .context("Failed to parse stdin as JSON")?;

    let auth_token = auth::read_token(&config.data_dir)
        .context("Failed to read auth token. Is the AgentToast daemon running?")?;

    // Antigravity payloads carry no event name, so the shape is the signal.
    let kind = AgyHookKind::from_payload(&payload);
    match kind {
        AgyHookKind::Stop => {
            return raise_toast(&config, &auth_token, adapter.parse_stop(&payload)?).await;
        }
        AgyHookKind::Unknown => {
            debug!("Payload is not an event AgentToast registers for");
            return Ok(());
        }
        AgyHookKind::PreToolUse => {}
    }

    let mut event = adapter
        .parse_hook_payload(&payload)
        .context("Failed to parse hook payload")?;
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

    // Bound the wait. Writing nothing and exiting 0 makes Antigravity fall back
    // to its own permission prompt, so a user who never answers the toast is
    // left deciding in the session rather than stuck forever.
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

    // What an approval has to carry for Antigravity to honour it: it has no
    // "a hook said yes" path, so the answer is a temporary permission grant
    // naming this exact call.
    let grants = agenttoast_adapters::agy::permission_grants(&payload);

    match response {
        IpcResponse::UserAction {
            action, text_input, ..
        } => match adapter.format_for_call(kind, action, text_input.as_deref(), grants)? {
            Some(hook_response) => {
                info!(response = %hook_response, "Sending decision to Antigravity");
                print!("{}", hook_response);
            }
            None => {
                info!("Handing the decision back to the session");
            }
        },
        IpcResponse::Error { message } => {
            error!(error = %message, "Daemon returned error");
        }
        IpcResponse::Ack { .. } => {
            debug!("Received ack instead of user action");
        }
    }

    Ok(())
}

/// Raise a toast for something Antigravity wants to say, and return at once.
///
/// Deliberately not blocking: the turn is over and nothing is waiting on an
/// answer, so holding the hook open would only delay the agent.
async fn raise_toast(
    config: &AppConfig,
    auth_token: &str,
    event: Option<agenttoast_core::event::AttentionEvent>,
) -> Result<()> {
    let Some(mut event) = event else {
        debug!("Nothing worth a toast");
        return Ok(());
    };
    event.process_id = agent_pid();

    info!(
        session_id = %event.session_id,
        message = %event.message,
        "Raising an unprompted toast"
    );

    let message = IpcMessage::Notify { event };
    let send = agenttoast_ipc::client::send_to_daemon(&config.ipc.pipe_name, auth_token, &message);

    // Never fail a turn over a toast nobody is waiting for.
    match tokio::time::timeout(NOTIFY_TIMEOUT, send).await {
        Ok(Ok(_)) => debug!("Toast raised"),
        Ok(Err(e)) => warn!(error = %e, "Could not reach AgentToast daemon"),
        Err(_) => warn!("Timed out raising the toast"),
    }
    Ok(())
}

/// PID of the Antigravity process this hook is running for.
///
/// Antigravity runs hook commands through `cmd /c`, so this binary's immediate
/// parent is a shell that exits the moment the hook returns — useless by the
/// time the user clicks "Open Session" seconds later. Walking up past the
/// shells finds the process that actually owns the terminal window.
fn agent_pid() -> Option<u32> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

    /// Intermediate processes between the agent and this bridge.
    const SHELLS: [&str; 5] = [
        "cmd.exe",
        "powershell.exe",
        "pwsh.exe",
        "sh.exe",
        "bash.exe",
    ];
    /// Enough to step over a shell or two without walking to the desktop.
    const MAX_HOPS: usize = 4;

    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing(),
    );

    let mut current = Pid::from_u32(std::process::id());
    let mut fallback = None;

    for _ in 0..MAX_HOPS {
        let parent = system.process(current).and_then(|p| p.parent())?;
        fallback.get_or_insert(parent);

        let name = system
            .process(parent)
            .map(|p| p.name().to_string_lossy().to_lowercase())
            .unwrap_or_default();

        if !SHELLS.contains(&name.as_str()) {
            return Some(parent.as_u32());
        }
        current = parent;
    }

    fallback.map(|p| p.as_u32())
}
