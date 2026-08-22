//! Session registry — tracks all active agent sessions.

use crate::event::AttentionEvent;
use crate::state::SessionState;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// The type of AI coding agent.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentType {
    ClaudeCode,
    Antigravity,
    Custom(String),
}

impl std::fmt::Display for AgentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentType::ClaudeCode => write!(f, "Claude Code"),
            AgentType::Antigravity => write!(f, "Antigravity"),
            AgentType::Custom(name) => write!(f, "{}", name),
        }
    }
}

/// An active agent session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub session_id: String,
    pub agent_type: AgentType,
    pub process_id: Option<u32>,
    pub working_directory: Option<String>,
    pub state: SessionState,
    pub created_at: DateTime<Utc>,
    pub last_activity_at: DateTime<Utc>,
    /// The current pending attention request, if any
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attention_request: Option<AttentionEvent>,
    /// Agent-specific metadata
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

impl Session {
    /// Create a new session.
    pub fn new(session_id: impl Into<String>, agent_type: AgentType) -> Self {
        let now = Utc::now();
        Self {
            session_id: session_id.into(),
            agent_type,
            process_id: None,
            working_directory: None,
            state: SessionState::Working,
            created_at: now,
            last_activity_at: now,
            attention_request: None,
            metadata: HashMap::new(),
        }
    }

    /// Update the session state and touch the last activity timestamp.
    pub fn update_state(&mut self, state: SessionState) {
        self.state = state;
        self.last_activity_at = Utc::now();
    }

    /// Set a pending attention request.
    pub fn set_attention(&mut self, event: AttentionEvent) {
        self.state = event.state;
        self.attention_request = Some(event);
        self.last_activity_at = Utc::now();
    }

    /// Clear the pending attention request.
    pub fn clear_attention(&mut self) {
        self.attention_request = None;
        self.state = SessionState::Working;
        self.last_activity_at = Utc::now();
    }

    /// How long this session has been waiting for attention.
    pub fn waiting_duration(&self) -> Option<chrono::Duration> {
        if self.state.needs_attention() {
            Some(Utc::now() - self.last_activity_at)
        } else {
            None
        }
    }
}

/// Thread-safe registry of all active sessions.
#[derive(Debug, Clone)]
pub struct SessionRegistry {
    sessions: Arc<RwLock<HashMap<String, Session>>>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a new session or update an existing one.
    pub async fn register(&self, session: Session) {
        let session_id = session.session_id.clone();
        let agent = session.agent_type.to_string();
        self.sessions.write().await.insert(session_id.clone(), session);
        info!(session_id = %session_id, agent = %agent, "Session registered");
    }

    /// Remove a session from the registry.
    pub async fn deregister(&self, session_id: &str) {
        if self.sessions.write().await.remove(session_id).is_some() {
            info!(session_id = %session_id, "Session deregistered");
        } else {
            warn!(session_id = %session_id, "Attempted to deregister unknown session");
        }
    }

    /// Get a clone of a session by ID.
    pub async fn get(&self, session_id: &str) -> Option<Session> {
        self.sessions.read().await.get(session_id).cloned()
    }

    /// Update a session's state.
    pub async fn update_state(&self, session_id: &str, state: SessionState) -> bool {
        if let Some(session) = self.sessions.write().await.get_mut(session_id) {
            session.update_state(state);
            true
        } else {
            false
        }
    }

    /// Set an attention request on a session.
    pub async fn set_attention(&self, session_id: &str, event: AttentionEvent) -> bool {
        if let Some(session) = self.sessions.write().await.get_mut(session_id) {
            session.set_attention(event);
            true
        } else {
            false
        }
    }

    /// Clear the attention request on a session.
    pub async fn clear_attention(&self, session_id: &str) -> bool {
        if let Some(session) = self.sessions.write().await.get_mut(session_id) {
            session.clear_attention();
            true
        } else {
            false
        }
    }

    /// Get all sessions.
    pub async fn all(&self) -> Vec<Session> {
        self.sessions.read().await.values().cloned().collect()
    }

    /// Get all sessions that need attention.
    pub async fn attention_needed(&self) -> Vec<Session> {
        self.sessions
            .read()
            .await
            .values()
            .filter(|s| s.state.needs_attention())
            .cloned()
            .collect()
    }

    /// Count of active sessions.
    pub async fn count(&self) -> usize {
        self.sessions.read().await.len()
    }
}

impl Default for SessionRegistry {
    fn default() -> Self {
        Self::new()
    }
}
