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
    /// Working directory of the session. Present on every hook event.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Which hook fired. Present on every event; used to tell a permission
    /// request apart from a session lifecycle notification.
    #[serde(default)]
    pub hook_event_name: Option<String>,
}

/// What a hook payload is asking AgentToast to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookKind {
    /// A tool wants permission — raise a toast and block on the answer.
    Attention,
    /// A session started — record it in the registry.
    SessionStart,
    /// A session ended — drop it from the registry.
    SessionEnd,
}

impl HookKind {
    /// Classify a payload by its `hook_event_name`.
    ///
    /// Anything unrecognised is treated as an attention request: that is the
    /// behaviour the hook config has always had, and erring toward showing a
    /// toast is better than silently swallowing an event.
    pub fn from_payload(payload: &Value) -> Self {
        match payload.get("hook_event_name").and_then(|v| v.as_str()) {
            Some("SessionStart") => HookKind::SessionStart,
            Some("SessionEnd") => HookKind::SessionEnd,
            _ => HookKind::Attention,
        }
    }
}

/// A `PreToolUse` hook result.
///
/// Claude Code takes the decision from `hookSpecificOutput.permissionDecision`;
/// the older top-level `decision` field is not read for this event, so emitting
/// it means the hook has no effect and the tool call just falls through to the
/// normal permission prompt.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeHookResponse {
    pub hook_specific_output: PreToolUseOutput,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreToolUseOutput {
    pub hook_event_name: &'static str,
    /// `allow`, `deny`, or `ask` (hand the decision back to the user).
    pub permission_decision: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_decision_reason: Option<String>,
}

impl ClaudeHookResponse {
    fn new(decision: &'static str, reason: Option<String>) -> Self {
        Self {
            hook_specific_output: PreToolUseOutput {
                hook_event_name: "PreToolUse",
                permission_decision: decision,
                permission_decision_reason: reason,
            },
        }
    }
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
        event.cwd = hook_data.cwd;

        Ok(event)
    }

    fn format_response(&self, action: ActionType, text: Option<&str>) -> Result<String> {
        let response = match action {
            ActionType::Approve | ActionType::Confirm => ClaudeHookResponse::new(
                "allow",
                Some("Approved by user via AgentToast".to_string()),
            ),
            ActionType::Deny | ActionType::Reject => ClaudeHookResponse::new(
                "deny",
                Some("Denied by user via AgentToast".to_string()),
            ),
            // The user chose to deal with it in the terminal instead, so hand
            // the decision back rather than silently answering on their behalf.
            // Must be "ask": the accepted values are allow / ask / deny, and
            // anything else fails Claude Code's output validation outright.
            ActionType::OpenSession => ClaudeHookResponse::new(
                "ask",
                Some("User opted to decide in the session".to_string()),
            ),
            ActionType::SendText => ClaudeHookResponse::new(
                "ask",
                text.map(|t| format!("User replied: {}", t)),
            ),
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Claude Code reads the decision from `hookSpecificOutput.permissionDecision`.
    /// A top-level `decision` field is ignored for PreToolUse, which would make
    /// every toast click a no-op, so pin the exact shape.
    #[test]
    fn approve_emits_permission_decision_allow() {
        let out = ClaudeAdapter.format_response(ActionType::Approve, None).unwrap();
        let parsed: Value = serde_json::from_str(&out).unwrap();

        assert_eq!(parsed["hookSpecificOutput"]["hookEventName"], "PreToolUse");
        assert_eq!(parsed["hookSpecificOutput"]["permissionDecision"], "allow");
        assert!(parsed.get("decision").is_none(), "legacy field must not be emitted");
    }

    #[test]
    fn deny_emits_permission_decision_deny_with_reason() {
        let out = ClaudeAdapter.format_response(ActionType::Deny, None).unwrap();
        let parsed: Value = serde_json::from_str(&out).unwrap();

        assert_eq!(parsed["hookSpecificOutput"]["permissionDecision"], "deny");
        assert!(parsed["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap()
            .contains("AgentToast"));
    }

    /// "Open session" means the user wants to decide in the terminal, so the
    /// decision goes back to them rather than being answered on their behalf.
    #[test]
    fn open_session_asks() {
        let out = ClaudeAdapter.format_response(ActionType::OpenSession, None).unwrap();
        let parsed: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed["hookSpecificOutput"]["permissionDecision"], "ask");
    }

    /// Claude Code validates this field against a fixed set and rejects the
    /// whole hook output otherwise — a wrong value shows up as
    /// "Hook JSON output validation failed", not as a silently ignored click.
    #[test]
    fn every_action_uses_an_accepted_decision() {
        const ACCEPTED: [&str; 3] = ["allow", "ask", "deny"];

        for action in [
            ActionType::Approve,
            ActionType::Deny,
            ActionType::Confirm,
            ActionType::Reject,
            ActionType::SendText,
            ActionType::OpenSession,
        ] {
            let out = ClaudeAdapter.format_response(action.clone(), None).unwrap();
            let parsed: Value = serde_json::from_str(&out).unwrap();
            let decision = parsed["hookSpecificOutput"]["permissionDecision"]
                .as_str()
                .unwrap()
                .to_string();
            assert!(
                ACCEPTED.contains(&decision.as_str()),
                "{:?} produced unaccepted decision {:?}",
                action,
                decision
            );
        }
    }

    #[test]
    fn bash_payload_becomes_permission_request() {
        let payload = json!({
            "session_id": "s1",
            "tool_name": "Bash",
            "tool_input": { "command": "npm run migrate" }
        });

        let event = ClaudeAdapter.parse_hook_payload(&payload).unwrap();

        assert_eq!(event.session_id, "s1");
        assert_eq!(event.tool_name.as_deref(), Some("Bash"));
        assert_eq!(event.message, "Claude wants to run: npm run migrate");
        assert_eq!(event.state, agenttoast_core::SessionState::WaitingForPermission);
    }
}
