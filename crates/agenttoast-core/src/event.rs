//! Attention events — the core data model for agent notifications.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// An event indicating that an agent session needs human attention.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttentionEvent {
    /// Unique identifier for this event
    pub event_id: Uuid,
    /// The session that generated this event
    pub session_id: String,
    /// The type of agent
    pub agent: String,
    /// Current state of the session
    pub state: crate::SessionState,
    /// Human-readable message describing what the agent needs
    pub message: String,
    /// Optional detailed context (e.g., the full command to be executed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// Available actions the user can take
    pub actions: Vec<Action>,
    /// When this event was created
    pub timestamp: DateTime<Utc>,
    /// Tool name that triggered this event (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// Working directory of the agent session, when the hook reports one.
    /// Lets the daemon register a session with somewhere to "open" it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// PID of the agent process that triggered this event.
    ///
    /// Carried on the event rather than looked up later so "Open Session" works
    /// even when the SessionStart hook is not configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_id: Option<u32>,
}

/// An action button that can be presented to the user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Action {
    /// The type of action
    #[serde(rename = "type")]
    pub action_type: ActionType,
    /// Display label for the button
    pub label: String,
}

/// Supported action types.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionType {
    Approve,
    Deny,
    Confirm,
    Reject,
    SendText,
    OpenSession,
}

/// The user's response to an attention event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserResponse {
    /// The event this is responding to
    pub event_id: Uuid,
    /// The action the user chose
    pub action: ActionType,
    /// Optional text input (for SendText actions)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_input: Option<String>,
}

impl AttentionEvent {
    /// Create a new permission request event (approve/deny).
    pub fn permission_request(
        session_id: impl Into<String>,
        agent: impl Into<String>,
        message: impl Into<String>,
        tool_name: Option<String>,
    ) -> Self {
        Self {
            event_id: Uuid::new_v4(),
            session_id: session_id.into(),
            agent: agent.into(),
            state: crate::SessionState::WaitingForPermission,
            message: message.into(),
            context: None,
            actions: vec![
                Action {
                    action_type: ActionType::Approve,
                    label: "Approve".into(),
                },
                Action {
                    action_type: ActionType::Deny,
                    label: "Deny".into(),
                },
                Action {
                    action_type: ActionType::OpenSession,
                    label: "Open Session".into(),
                },
            ],
            timestamp: Utc::now(),
            tool_name,
            cwd: None,
            process_id: None,
        }
    }

    /// Create a new confirmation request event (yes/no).
    pub fn confirmation_request(
        session_id: impl Into<String>,
        agent: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            event_id: Uuid::new_v4(),
            session_id: session_id.into(),
            agent: agent.into(),
            state: crate::SessionState::WaitingForConfirmation,
            message: message.into(),
            context: None,
            actions: vec![
                Action {
                    action_type: ActionType::Confirm,
                    label: "Yes".into(),
                },
                Action {
                    action_type: ActionType::Reject,
                    label: "No".into(),
                },
                Action {
                    action_type: ActionType::OpenSession,
                    label: "Open Session".into(),
                },
            ],
            timestamp: Utc::now(),
            tool_name: None,
            cwd: None,
            process_id: None,
        }
    }
}
