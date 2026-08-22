//! Claude Code adapter.

use crate::AgentAdapter;
use agenttoast_core::event::{ActionType, AttentionEvent};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::debug;

pub struct ClaudeAdapter;

#[derive(Debug, Deserialize)]
pub struct ClaudeHookPayload {
    pub session_id: String,
    #[serde(default)]
    pub transcript_path: Option<String>,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub tool_input: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct ClaudeHookResponse {
    pub decision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl AgentAdapter for ClaudeAdapter {
    fn agent_id(&self) -> &str {
        "claude"
    }

    fn parse_hook_payload(&self, payload: &Value) -> Result<AttentionEvent> {
        let hook_data: ClaudeHookPayload = serde_json::from_value(payload.clone())
            .context("Failed to parse Claude Code hook payload")?;

        let tool_name = hook_data.tool_name.clone().unwrap_or_default();
        let message = build_message(&tool_name, &hook_data.tool_input);

        debug!(
            session_id = %hook_data.session_id,
            tool = %tool_name,
            "Parsed Claude Code hook payload"
        );

        let mut event = AttentionEvent::permission_request(
            &hook_data.session_id,
            "claude",
            &message,
            Some(tool_name),
        );

        if let Some(input) = &hook_data.tool_input {
            event.context = Some(serde_json::to_string_pretty(input).unwrap_or_default());
        }

        Ok(event)
    }

    fn format_response(&self, action: ActionType, _text: Option<&str>) -> Result<String> {
        let response = match action {
            ActionType::Approve | ActionType::Confirm => ClaudeHookResponse {
                decision: "approve".to_string(),
                reason: None,
            },
            ActionType::Deny | ActionType::Reject => ClaudeHookResponse {
                decision: "deny".to_string(),
                reason: Some("Denied by user via AgentToast".to_string()),
            },
            ActionType::OpenSession => {
                ClaudeHookResponse {
                    decision: "approve".to_string(),
                    reason: None,
                }
            }
            ActionType::SendText => ClaudeHookResponse {
                decision: "approve".to_string(),
                reason: None,
            },
        };

        serde_json::to_string(&response).context("Failed to serialize Claude hook response")
    }

    fn extract_session_id(&self, payload: &Value) -> Result<String> {
        payload
            .get("session_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .context("Missing 'session_id' in Claude Code hook payload")
    }
}

fn build_message(tool_name: &str, tool_input: &Option<Value>) -> String {
    match tool_name {
        "Bash" => {
            if let Some(input) = tool_input {
                if let Some(cmd) = input.get("command").and_then(|v| v.as_str()) {
                    return format!("Claude wants to run: {}", cmd);
                }
            }
            "Claude wants to run a command".to_string()
        }
        "Write" => {
            if let Some(input) = tool_input {
                if let Some(path) = input.get("file_path").and_then(|v| v.as_str()) {
                    return format!("Claude wants to write to: {}", path);
                }
            }
            "Claude wants to write a file".to_string()
        }
        "Edit" => {
            if let Some(input) = tool_input {
                if let Some(path) = input.get("file_path").and_then(|v| v.as_str()) {
                    return format!("Claude wants to edit: {}", path);
                }
            }
            "Claude wants to edit a file".to_string()
        }
        _ => format!("Claude wants to use tool: {}", tool_name),
    }
}
