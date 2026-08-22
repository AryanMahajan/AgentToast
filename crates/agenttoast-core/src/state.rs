//! Session state machine.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Possible states of an agent session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SessionState {
    /// Agent is actively working
    Working,
    /// Agent is waiting for text input from the user
    WaitingForInput,
    /// Agent is waiting for permission to execute a tool
    WaitingForPermission,
    /// Agent is waiting for a yes/no confirmation
    WaitingForConfirmation,
    /// Agent encountered an error
    Error,
    /// Agent finished its task
    Completed,
    /// Agent is idle (no active task)
    Idle,
}

impl SessionState {
    /// Whether this state requires human attention.
    pub fn needs_attention(&self) -> bool {
        matches!(
            self,
            SessionState::WaitingForInput
                | SessionState::WaitingForPermission
                | SessionState::WaitingForConfirmation
                | SessionState::Error
        )
    }

    /// Whether this state indicates the agent is actively working.
    pub fn is_active(&self) -> bool {
        matches!(self, SessionState::Working)
    }

    /// Whether this state is terminal.
    pub fn is_terminal(&self) -> bool {
        matches!(self, SessionState::Completed | SessionState::Error)
    }
}

impl fmt::Display for SessionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionState::Working => write!(f, "Working"),
            SessionState::WaitingForInput => write!(f, "Waiting for input"),
            SessionState::WaitingForPermission => write!(f, "Waiting for permission"),
            SessionState::WaitingForConfirmation => write!(f, "Waiting for confirmation"),
            SessionState::Error => write!(f, "Error"),
            SessionState::Completed => write!(f, "Completed"),
            SessionState::Idle => write!(f, "Idle"),
        }
    }
}
