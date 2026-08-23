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
        match pending_event(&state, event_uuid).await {
            Some(event) => {
                crate::focus::focus_agent_window(event.process_id.unwrap_or_default());
            }
            None => {
                tracing::warn!(
                    event_id = %event_id,
                    "Request is no longer pending; nothing to raise"
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

/// Take a toast off screen without answering it.
///
/// Closing a toast is "not now", not "decide without me": the request stays
/// pending and the agent stays blocked until it is answered here or from the
/// dashboard.
#[tauri::command]
pub fn hide_toast(app: tauri::AppHandle, event_id: String) {
    crate::window::hide_toast(&app, &event_id);
}

/// Close the toast window from the frontend.
#[tauri::command]
pub fn close_window(
    app: tauri::AppHandle,
    event_id: String,
) -> Result<(), String> {
    crate::window::close_toast(&app, &event_id).map_err(|e| e.to_string())
}

/// Re-open the toast for a request that was hidden.
///
/// Hiding a toast leaves the request pending and the agent blocked, so there
/// has to be a way to pull it back without answering from the dashboard.
#[tauri::command]
pub async fn reopen_toast(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    event_id: String,
) -> Result<(), String> {
    let event_uuid = Uuid::parse_str(&event_id).map_err(|e| format!("Invalid event ID: {}", e))?;

    let event = state
        .sessions
        .all()
        .await
        .into_iter()
        .filter_map(|s| s.attention_request)
        .find(|e| e.event_id == event_uuid)
        .ok_or_else(|| "That request is no longer pending".to_string())?;

    crate::window::restore(&app, &event);
    Ok(())
}

/// The still-pending attention event with this id, if any.
async fn pending_event(state: &Arc<AppState>, event_id: Uuid) -> Option<AttentionEvent> {
    state
        .sessions
        .all()
        .await
        .into_iter()
        .filter_map(|s| s.attention_request)
        .find(|e| e.event_id == event_id)
}
