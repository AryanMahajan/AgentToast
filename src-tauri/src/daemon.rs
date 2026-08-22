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
                    let agent_type = match agent.as_str() {
                        "claude" => AgentType::ClaudeCode,
                        "agy" => AgentType::Antigravity,
                        other => AgentType::Custom(other.to_string()),
                    };
                    let mut session = Session::new(&session_id, agent_type);
                    session.process_id = process_id;
                    session.working_directory = working_directory;
                    state.sessions.register(session).await;
                    let _ = incoming.response_tx.send(IpcResponse::ack());
                }

                IpcMessage::Attention { event } => {
                    handle_attention(&app, &state, event, incoming.response_tx).await;
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

    // Auto-register session if not already known
    if state.sessions.get(&session_id).await.is_none() {
        let agent_type = match event.agent.as_str() {
            "claude" => AgentType::ClaudeCode,
            "agy" => AgentType::Antigravity,
            other => AgentType::Custom(other.to_string()),
        };
        state
            .sessions
            .register(Session::new(&session_id, agent_type))
            .await;
    }

    // Mark session as needing attention
    state.sessions.set_attention(&session_id, event.clone()).await;

    // Show the toast window
    if let Err(e) = window::show_toast(app, &event) {
        error!(error = %e, "Failed to show toast window");
    }

    // Emit event to frontend
    let _ = app.emit("attention-event", &event);

    // Wait for the user's response (this blocks until the user clicks a button)
    match response_rx.await {
        Ok(user_response) => {
            info!(
                event_id = %event_id,
                action = ?user_response.action,
                "User responded to attention event"
            );

            // Clear the attention state
            state.sessions.clear_attention(&session_id).await;

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
