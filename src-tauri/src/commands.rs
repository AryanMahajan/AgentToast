//! Tauri commands — invokable from the frontend JavaScript.

use crate::AppState;
use agenttoast_core::event::{ActionType, AttentionEvent, UserResponse};
use agenttoast_core::session::Session;
use std::sync::Arc;
use uuid::Uuid;

/// Get all active sessions.
#[tauri::command]
pub async fn get_sessions(state: tauri::State<'_, Arc<AppState>>) -> Result<Vec<Session>, String> {
    Ok(state.sessions.all().await)
}

/// Get the one attention event a toast window is responsible for.
///
/// Each toast window carries its event id in its label, so it asks for its own
/// event by id rather than guessing at the first pending one — otherwise two
/// concurrent toasts both render whichever event happens to come back first.
#[tauri::command]
pub async fn get_event(
    state: tauri::State<'_, Arc<AppState>>,
    event_id: String,
) -> Result<Option<AttentionEvent>, String> {
    let event_uuid = Uuid::parse_str(&event_id).map_err(|e| format!("Invalid event ID: {}", e))?;

    Ok(state
        .sessions
        .all()
        .await
        .into_iter()
        .filter_map(|s| s.attention_request)
        .find(|e| e.event_id == event_uuid))
}

/// Reveal a toast once its frontend has painted and measured itself.
#[tauri::command]
pub fn toast_ready(app: tauri::AppHandle, event_id: String, height: f64) {
    crate::window::mark_ready(&app, &event_id, height);
}

/// Get all sessions with pending attention events.
#[tauri::command]
pub async fn get_pending_events(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<Session>, String> {
    Ok(state.sessions.attention_needed().await)
}

/// Respond to an attention event (approve, deny, etc.).
#[tauri::command]
pub async fn respond_to_event(
    state: tauri::State<'_, Arc<AppState>>,
    event_id: String,
    action: String,
    text_input: Option<String>,
) -> Result<(), String> {
    let event_uuid = Uuid::parse_str(&event_id).map_err(|e| format!("Invalid event ID: {}", e))?;

    let action_type = match action.as_str() {
        "approve" => ActionType::Approve,
        "deny" => ActionType::Deny,
        "confirm" => ActionType::Confirm,
        "reject" => ActionType::Reject,
        "send_text" => ActionType::SendText,
        "open_session" => ActionType::OpenSession,
        _ => return Err(format!("Unknown action: {}", action)),
    };

    // "Open session" means take me to the agent, so raise its terminal before
    // handing the decision back. Do this first: resolving the event unblocks
    // the bridge, which clears the session's pending request.
    if action_type == ActionType::OpenSession {
        match session_pid_for_event(&state, event_uuid).await {
            Some(pid) => {
                crate::focus::focus_agent_window(pid);
            }
            None => {
                tracing::warn!(
                    event_id = %event_id,
                    "No pid recorded for this session; cannot raise its terminal"
                );
            }
        }
    }

    let response = UserResponse {
        event_id: event_uuid,
        action: action_type,
        text_input,
    };

    state
        .router
        .resolve(response)
        .await
        .map_err(|e| format!("Failed to resolve event: {}", e))?;

    Ok(())
}

/// Dismiss an event without responding (timeout or user closed toast).
#[tauri::command]
pub async fn dismiss_event(
    state: tauri::State<'_, Arc<AppState>>,
    event_id: String,
) -> Result<(), String> {
    let event_uuid = Uuid::parse_str(&event_id).map_err(|e| format!("Invalid event ID: {}", e))?;
    state.router.cancel(&event_uuid).await;
    Ok(())
}

/// Close the toast window from the frontend.
#[tauri::command]
pub fn close_window(
    app: tauri::AppHandle,
    event_id: String,
) -> Result<(), String> {
    crate::window::close_toast(&app, &event_id).map_err(|e| e.to_string())
}

/// PID of the agent process that owns the session behind `event_id`.
async fn session_pid_for_event(state: &Arc<AppState>, event_id: Uuid) -> Option<u32> {
    state
        .sessions
        .all()
        .await
        .into_iter()
        .find(|s| {
            s.attention_request
                .as_ref()
                .is_some_and(|e| e.event_id == event_id)
        })
        .and_then(|s| s.process_id)
}
