//! Tauri commands — invokable from the frontend JavaScript.

use crate::AppState;
use agenttoast_core::event::{ActionType, UserResponse};
use agenttoast_core::session::Session;
use std::sync::Arc;
use uuid::Uuid;

/// Get all active sessions.
#[tauri::command]
pub async fn get_sessions(state: tauri::State<'_, Arc<AppState>>) -> Result<Vec<Session>, String> {
    Ok(state.sessions.all().await)
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
