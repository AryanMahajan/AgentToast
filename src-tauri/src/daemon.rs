//! IPC daemon — processes incoming events from bridge scripts.

use crate::AppState;
use agenttoast_core::event::AttentionEvent;
use agenttoast_core::session::{AgentType, Session};
use agenttoast_ipc::protocol::{IpcMessage, IpcResponse};
use agenttoast_ipc::server;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tracing::{error, info, warn};

use crate::window;

/// Start the IPC daemon that listens for bridge connections.
pub async fn start(app_handle: AppHandle) {
    let state = app_handle.state::<Arc<AppState>>();

    let mut event_rx = match server::start_server(&state.config, &state.auth_token).await {
        Ok(rx) => rx,
        Err(e) => {
            error!(error = %e, "Failed to start IPC server");
            return;
        }
    };

    info!("IPC daemon started, waiting for events...");

    while let Some(incoming) = event_rx.recv().await {
        let app = app_handle.clone();

        tokio::spawn(async move {
            let state = app.state::<Arc<AppState>>();

            match incoming.message {
                IpcMessage::Register {
                    agent,
                    session_id,
                    process_id,
                    working_directory,
                } => {
                    let mut session = Session::new(&session_id, agent_type(&agent));
                    session.process_id = process_id;
                    session.working_directory = working_directory;
                    state.sessions.register(session).await;
                    let _ = incoming.response_tx.send(IpcResponse::ack());
                }

                IpcMessage::Attention { event } => {
                    handle_attention(&app, &state, event, incoming.response_tx).await;
                }

                IpcMessage::Notify { event } => {
                    // Nothing is waiting on this one, so acknowledge before
                    // showing anything: the agent should not be held up by a
                    // toast it cannot receive an answer from.
                    let _ = incoming.response_tx.send(IpcResponse::ack());
                    show_notification(&app, &state, event).await;
                }

                IpcMessage::Deregister { session_id } => {
                    state.sessions.deregister(&session_id).await;
                    let _ = incoming.response_tx.send(IpcResponse::ack());
                }

                IpcMessage::Ping { session_id } => {
                    state
                        .sessions
                        .update_state(
                            &session_id,
                            agenttoast_core::state::SessionState::Working,
                        )
                        .await;
                    let _ = incoming.response_tx.send(IpcResponse::ack());
                }
            }
        });
    }
}

/// The agent an event or registration came from.
///
/// Every bridge reports its own id, so a session ends up labelled by whichever
/// agent actually created it rather than by whichever one was assumed.
fn agent_type(agent: &str) -> AgentType {
    match agent {
        "claude" => AgentType::ClaudeCode,
        "agy" => AgentType::Antigravity,
        other => AgentType::Custom(other.to_string()),
    }
}

/// Handle an attention event: show the toast and wait for user response.
async fn handle_attention(
    app: &AppHandle,
    state: &Arc<AppState>,
    event: AttentionEvent,
    response_tx: tokio::sync::oneshot::Sender<IpcResponse>,
) {
    let event_id = event.event_id;
    let session_id = event.session_id.clone();

    info!(
        event_id = %event_id,
        session_id = %session_id,
        message = %event.message,
        "Received attention event"
    );

    // Register the event and get a channel that will receive the user's response
    let response_rx = state.router.register(&event).await;

    // Auto-register the session if the SessionStart hook is not configured, or
    // if the daemon was started midway through a session.
    if state.sessions.get(&session_id).await.is_none() {
        let mut session = Session::new(&session_id, agent_type(&event.agent));
        session.working_directory = event.cwd.clone();
        session.process_id = event.process_id;
        state.sessions.register(session).await;
    }

    // Mark session as needing attention
    state.sessions.set_attention(&session_id, event.clone()).await;

    // Show the toast window
    if let Err(e) = window::show_toast(app, &event) {
        error!(error = %e, "Failed to show toast window");
    }

    // Emit event to frontend
    let _ = app.emit("attention-event", &event);

    // Wait for the user's response, nudging them on the configured schedule
    // while the agent stays blocked. `oneshot::Receiver` is Unpin, so it can be
    // polled by reference across loop iterations.
    let escalation = state.config.escalation.clone();
    let mut response_rx = response_rx;
    let mut reminders_sent: u32 = 0;

    let outcome = loop {
        let wait = escalation.interval_for_reminder(reminders_sent as usize);

        tokio::select! {
            result = &mut response_rx => break result,
            _ = tokio::time::sleep(wait), if escalation.should_remind(reminders_sent) => {
                reminders_sent += 1;
                info!(
                    event_id = %event_id,
                    reminder = reminders_sent,
                    "Re-surfacing unanswered attention event"
                );
                window::remind(app, &event, escalation.sound_on_reminder);
            }
        }
    };

    match outcome {
        Ok(user_response) => {
            info!(
                event_id = %event_id,
                action = ?user_response.action,
                "User responded to attention event"
            );

            // Clear the attention state
            state.sessions.clear_attention(&session_id).await;

            // The answer may have come from the dashboard, leaving this event's
            // toast hidden but alive. Nothing is waiting on it now.
            let _ = window::close_toast(app, &event_id.to_string());

            // Send response back to the bridge
            let ipc_response = IpcResponse::UserAction {
                event_id,
                action: user_response.action,
                text_input: user_response.text_input,
            };
            let _ = response_tx.send(ipc_response);
        }
        Err(_) => {
            warn!(event_id = %event_id, "Response channel dropped (event cancelled or timed out)");
            state.sessions.clear_attention(&session_id).await;
            let _ = response_tx.send(IpcResponse::error("Event cancelled"));
        }
    }
}

/// Show a toast for something the agent wants to say.
///
/// No router entry and no waiting: the toast is informational, and its only
/// action takes the user to the session where they can actually answer.
async fn show_notification(app: &AppHandle, state: &Arc<AppState>, event: AttentionEvent) {
    let session_id = event.session_id.clone();

    info!(
        event_id = %event.event_id,
        session_id = %session_id,
        message = %event.message,
        "Received notification"
    );

    if state.sessions.get(&session_id).await.is_none() {
        let mut session = Session::new(&session_id, agent_type(&event.agent));
        session.working_directory = event.cwd.clone();
        session.process_id = event.process_id;
        state.sessions.register(session).await;
    }

    state.sessions.set_attention(&session_id, event.clone()).await;

    if let Err(e) = window::show_toast(app, &event) {
        error!(error = %e, "Failed to show notification toast");
    }
    let _ = app.emit("attention-event", &event);
}
