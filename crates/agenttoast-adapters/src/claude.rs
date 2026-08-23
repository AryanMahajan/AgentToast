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
    /// Claude Code is asking the user for permission. This is the event
    /// AgentToast exists for: it fires only once Claude has decided it needs a
    /// human, so it already respects whatever permission mode is in force.
    PermissionRequest,
    /// A tool is about to run. Fires for *every* matching call, whether or not
    /// Claude Code would have asked — so in `auto` or `acceptEdits` it
    /// overrides a choice the user deliberately made. Supported for anyone who
    /// wants a toast on everything, but not what gets configured by default.
    PreToolUse,
    /// A session started — record it in the registry.
    SessionStart,
    /// A session ended — drop it from the registry.
    SessionEnd,
}

impl HookKind {
    /// Classify a payload by its `hook_event_name`.
    ///
    /// Anything unrecognised is treated as a permission request: erring toward
    /// showing a toast beats silently swallowing an event.
    pub fn from_payload(payload: &Value) -> Self {
        match payload.get("hook_event_name").and_then(|v| v.as_str()) {
            Some("SessionStart") => HookKind::SessionStart,
            Some("SessionEnd") => HookKind::SessionEnd,
            Some("PreToolUse") => HookKind::PreToolUse,
            _ => HookKind::PermissionRequest,
        }
    }

    /// Whether this event should raise a toast and wait for an answer.
    pub fn needs_attention(self) -> bool {
        matches!(self, HookKind::PermissionRequest | HookKind::PreToolUse)
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

/// A `PermissionRequest` hook result.
///
/// A different shape from `PreToolUse`: the decision is an object with a
/// `behavior` of `allow` or `deny`, and there is no `ask` — declining to decide
/// is expressed by writing nothing at all, which lets Claude Code fall through
/// to its own prompt.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRequestResponse {
    pub hook_specific_output: PermissionRequestOutput,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRequestOutput {
    pub hook_event_name: &'static str,
    pub decision: PermissionDecision,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionDecision {
    /// `allow` or `deny`.
    pub behavior: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl PermissionRequestResponse {
    fn new(behavior: &'static str, message: Option<String>) -> Self {
        Self {
            hook_specific_output: PermissionRequestOutput {
                hook_event_name: "PermissionRequest",
                decision: PermissionDecision { behavior, message },
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
    // Claude writes its own one-line summary of what it is about to do —
    // "Remove hello.txt" rather than the full shell line. It is better than
    // anything reconstructed from the raw input, so prefer it when present.
    // The raw command still reaches the toast through the context field.
    if let Some(description) = tool_input
        .as_ref()
        .and_then(|input| input.get("description"))
        .and_then(|d| d.as_str())
        .map(str::trim)
        .filter(|d| !d.is_empty())
    {
        return description.to_string();
    }

    build_message_from_input(tool_name, tool_input)
}

fn build_message_from_input(tool_name: &str, tool_input: &Option<Value>) -> String {
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

impl ClaudeAdapter {
    /// Format a user's answer for whichever hook asked the question.
    ///
    /// `None` means write nothing: for a `PermissionRequest` there is no way to
    /// say "ask the user instead", so staying silent is how the decision is
    /// handed back to Claude Code's own prompt.
    pub fn format_for(
        &self,
        kind: HookKind,
        action: ActionType,
        text: Option<&str>,
    ) -> Result<Option<String>> {
        match kind {
            HookKind::PreToolUse => self.format_response(action, text).map(Some),
            HookKind::PermissionRequest => self.format_permission_request(action),
            // Lifecycle events carry no decision.
            HookKind::SessionStart | HookKind::SessionEnd => Ok(None),
        }
    }

    fn format_permission_request(&self, action: ActionType) -> Result<Option<String>> {
        let response = match action {
            ActionType::Approve | ActionType::Confirm => {
                PermissionRequestResponse::new("allow", None)
            }
            ActionType::Deny | ActionType::Reject => PermissionRequestResponse::new(
                "deny",
                Some("Denied by user via AgentToast".to_string()),
            ),
            // The user wants to decide in the terminal. Saying nothing lets
            // Claude Code ask them there, which is exactly what they asked for.
            ActionType::OpenSession | ActionType::SendText => return Ok(None),
        };

        serde_json::to_string(&response)
            .map(Some)
            .context("Failed to serialize Claude permission response")
    }
}

#[cfg(test)]
mod permission_request_tests {
    use super::*;
    use serde_json::json;

    fn decision(action: ActionType) -> Option<Value> {
        ClaudeAdapter
            .format_for(HookKind::PermissionRequest, action, None)
            .unwrap()
            .map(|raw| serde_json::from_str(&raw).unwrap())
    }

    #[test]
    fn permission_request_is_classified_by_event_name() {
        let payload = json!({ "hook_event_name": "PermissionRequest" });
        assert_eq!(HookKind::from_payload(&payload), HookKind::PermissionRequest);

        let payload = json!({ "hook_event_name": "PreToolUse" });
        assert_eq!(HookKind::from_payload(&payload), HookKind::PreToolUse);
    }

    /// The shape differs from PreToolUse: a decision object with `behavior`,
    /// not a `permissionDecision` string.
    #[test]
    fn approve_allows_via_behavior() {
        let out = decision(ActionType::Approve).expect("approve must answer");
        assert_eq!(out["hookSpecificOutput"]["hookEventName"], "PermissionRequest");
        assert_eq!(out["hookSpecificOutput"]["decision"]["behavior"], "allow");
        assert!(
            out["hookSpecificOutput"].get("permissionDecision").is_none(),
            "the PreToolUse field must not leak into this response"
        );
    }

    #[test]
    fn deny_blocks_with_a_reason() {
        let out = decision(ActionType::Deny).expect("deny must answer");
        assert_eq!(out["hookSpecificOutput"]["decision"]["behavior"], "deny");
        assert!(out["hookSpecificOutput"]["decision"]["message"]
            .as_str()
            .unwrap()
            .contains("AgentToast"));
    }

    /// There is no "ask" for this event, so declining to decide means writing
    /// nothing at all and letting Claude Code prompt in the terminal.
    #[test]
    fn open_session_answers_nothing() {
        assert!(decision(ActionType::OpenSession).is_none());
        assert!(decision(ActionType::SendText).is_none());
    }

    #[test]
    fn lifecycle_events_never_answer() {
        for kind in [HookKind::SessionStart, HookKind::SessionEnd] {
            let out = ClaudeAdapter
                .format_for(kind, ActionType::Approve, None)
                .unwrap();
            assert!(out.is_none(), "{:?} must not emit a decision", kind);
        }
    }

    #[test]
    fn claudes_own_description_becomes_the_headline() {
        let payload = json!({
            "session_id": "s1",
            "hook_event_name": "PermissionRequest",
            "tool_name": "Bash",
            "tool_input": {
                "command": "rm \"C:/Users/DELL/Desktop/toast-test/hello.txt\" && echo \"deleted\"",
                "description": "Remove hello.txt"
            }
        });

        let event = ClaudeAdapter.parse_hook_payload(&payload).unwrap();
        assert_eq!(event.message, "Remove hello.txt");
        // The full command is still available for the toast's detail line.
        assert!(event.context.unwrap().contains("rm "));
    }

    #[test]
    fn falls_back_to_the_command_when_there_is_no_description() {
        let payload = json!({
            "session_id": "s1",
            "tool_name": "Bash",
            "tool_input": { "command": "npm run migrate" }
        });

        let event = ClaudeAdapter.parse_hook_payload(&payload).unwrap();
        assert_eq!(event.message, "Claude wants to run: npm run migrate");
    }
}
