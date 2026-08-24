//! AgentToast Adapters
//!
//! Agent-specific adapters that translate between each agent's hook
//! payload format and AgentToast's generic event model.

pub mod agy;
pub mod claude;

use agenttoast_core::event::{ActionType, AttentionEvent};
use anyhow::Result;
use serde_json::Value;

/// Trait that each agent adapter must implement.
///
/// An adapter translates between the agent's hook JSON format
/// and AgentToast's generic AttentionEvent model.
pub trait AgentAdapter: Send + Sync {
    /// The agent identifier string (e.g., "claude", "agy")
    fn agent_id(&self) -> &str;

    /// Parse the agent's hook stdin payload into an AttentionEvent.
    fn parse_hook_payload(&self, payload: &Value) -> Result<AttentionEvent>;

    /// Format a user action into the agent's expected hook stdout response.
    fn format_response(&self, action: ActionType, text: Option<&str>) -> Result<String>;

    /// Extract the session ID from the hook payload.
    fn extract_session_id(&self, payload: &Value) -> Result<String>;
}
