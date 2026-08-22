//! IPC message protocol — JSON messages exchanged between bridges and the daemon.

use agenttoast_core::event::{ActionType, AttentionEvent, UserResponse};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Messages sent FROM bridge scripts TO the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IpcMessage {
    /// Register a new agent session
    Register {
        agent: String,
        session_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        process_id: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        working_directory: Option<String>,
    },
    /// An agent needs attention
    Attention {
        #[serde(flatten)]
        event: AttentionEvent,
    },
    /// Deregister a session
    Deregister {
        session_id: String,
    },
    /// Heartbeat / keep-alive
    Ping {
        session_id: String,
    },
}

/// Responses sent FROM the daemon TO bridge scripts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IpcResponse {
    /// Acknowledgment (for register, deregister, ping)
    Ack {
        success: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// User's response to an attention event
    UserAction {
        event_id: Uuid,
        action: ActionType,
        #[serde(skip_serializing_if = "Option::is_none")]
        text_input: Option<String>,
    },
    /// Error response
    Error {
        message: String,
    },
}

impl IpcResponse {
    /// Create a success acknowledgment.
    pub fn ack() -> Self {
        Self::Ack {
            success: true,
            message: None,
        }
    }

    /// Create an error response.
    pub fn error(msg: impl Into<String>) -> Self {
        Self::Error {
            message: msg.into(),
        }
    }

    /// Convert a UserAction response into a UserResponse.
    pub fn into_user_response(self) -> Option<UserResponse> {
        match self {
            IpcResponse::UserAction {
                event_id,
                action,
                text_input,
            } => Some(UserResponse {
                event_id,
                action,
                text_input,
            }),
            _ => None,
        }
    }
}
