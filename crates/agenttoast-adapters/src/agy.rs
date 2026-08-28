//! Antigravity (`agy`) adapter.
//!
//! Antigravity's hook system looks superficially like Claude Code's — there is
//! even a `PreToolUse` event with the same name — but nothing about the wire
//! format is shared. The payload is protojson (camelCase) rather than
//! snake_case, the decision goes at the top level rather than inside
//! `hookSpecificOutput`, and there is no event name in the payload at all, so
//! events are told apart by shape.
//!
//! Everything encoded here was read off the shipped `agy` binary and then
//! confirmed by running it against a logging hook, because the two disagree in
//! places — the documented "lowercase the step type" rule for tool names, for
//! instance, gives `list_directory` where the tool is really called `list_dir`.

use agenttoast_core::event::{Action, ActionType, AttentionEvent};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::debug;

pub struct AgyAdapter;

/// The agent id carried on events, matching `AgentType::Antigravity`.
pub const AGENT_ID: &str = "agy";

/// Tools worth a toast by default.
///
/// Antigravity has no equivalent of Claude Code's `PermissionRequest` — there
/// is no event that means "the agent has decided it needs a human". `PreToolUse`
/// fires for every tool that matches, whether or not Antigravity would have
/// prompted, so the matcher is the only way to keep this from toasting on every
/// directory listing. These are the calls that change something.
pub const DEFAULT_MATCHER: &str =
    "^(run_command|write_to_file|create_file|replace_file_content|edit_notebook|delete_file)$";

/// Shell commands only.
///
/// The narrow half of [`Watch`], for a session that does not want file-edit
/// toasts.
pub const COMMAND_MATCHER: &str = "^run_command$";

/// Which calls raise a toast.
///
/// This exists because Antigravity gates commands and file edits through two
/// different mechanisms, and only one of them is a thing a hook can stand in
/// front of honestly.
///
/// - **Commands** are governed by the permission lists, in every execution mode.
///   A toast for `run_command` is therefore always right.
/// - **File edits** are governed by the *execution mode*: `default` pauses for a
///   line-level diff review, and `accept-edits` deliberately does not pause at
///   all. A toast is right in the first and plainly wrong in the second — it
///   reintroduces exactly the interruption the mode was chosen to remove.
///
/// And the mode cannot be detected. `HookArgsCommon` carries `conversationId`,
/// `workspacePaths`, `transcriptPath`, `artifactDirectoryPath`, `executionId`,
/// `modelName`, `isBattleMode` and `lastUserInput` — no execution mode. Reading
/// `agentMode` from `settings.json` would only give the startup default, which
/// `--mode` and Shift+Tab both override without writing anything down, so it
/// would be wrong precisely when someone had gone out of their way to change it.
///
/// So the choice is the user's, made once, and it decides both the matcher and
/// whether `write_file(*)` is granted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Watch {
    /// Commands and the calls that write to disk.
    CommandsAndEdits,
    /// Commands only — the right setting for an `accept-edits` session.
    CommandsOnly,
}

impl Watch {
    /// The `PreToolUse` matcher this scope writes into `hooks.json`.
    pub fn matcher(self) -> &'static str {
        match self {
            Watch::CommandsAndEdits => DEFAULT_MATCHER,
            Watch::CommandsOnly => COMMAND_MATCHER,
        }
    }

    /// Read back the scope a `hooks.json` matcher represents.
    ///
    /// Anything unrecognised — a matcher somebody edited by hand — counts as the
    /// wider scope, since that is the one whose extra grant needs withdrawing.
    pub fn from_matcher(matcher: &str) -> Self {
        if matcher == COMMAND_MATCHER {
            Watch::CommandsOnly
        } else {
            Watch::CommandsAndEdits
        }
    }

    pub fn watches_edits(self) -> bool {
        self == Watch::CommandsAndEdits
    }
}

/// What a hook payload is asking AgentToast to do.
///
/// Antigravity payloads carry no event name, so the shape is the only signal:
/// a tool call has `toolCall`, the end of an execution loop has
/// `terminationReason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgyHookKind {
    /// A tool is about to run, and the agent is blocked until this hook answers.
    PreToolUse,
    /// The execution loop is ending.
    Stop,
    /// Something we did not register for. Answered with silence.
    Unknown,
}

impl AgyHookKind {
    pub fn from_payload(payload: &Value) -> Self {
        if payload.get("toolCall").is_some() {
            AgyHookKind::PreToolUse
        } else if payload.get("terminationReason").is_some() {
            AgyHookKind::Stop
        } else {
            AgyHookKind::Unknown
        }
    }

    /// Whether this event blocks the agent until the user answers.
    pub fn needs_attention(self) -> bool {
        matches!(self, AgyHookKind::PreToolUse)
    }
}

/// A `PreToolUse` payload.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgyToolCallPayload {
    pub conversation_id: String,
    pub tool_call: AgyToolCall,
    #[serde(default)]
    pub workspace_paths: Vec<String>,
    #[serde(default)]
    pub step_idx: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct AgyToolCall {
    pub name: String,
    /// Tool arguments. Mostly PascalCase (`CommandLine`, `TargetFile`), with a
    /// few tools using snake_case, so this stays untyped.
    #[serde(default)]
    pub args: Option<Value>,
}

/// A `PreToolUse` hook result.
///
/// Flat, unlike Claude Code's nested `hookSpecificOutput`, and much smaller than
/// the message it is decoded into. `PreToolHookResult` also carries `allow_tool`,
/// `deny_reason`, `overwrite` and `permission_overrides`; none of them are sent,
/// because none of them change what Antigravity does with a call.
///
/// **A hook cannot grant permission.** `decision: "allow"` is read, parsed
/// without complaint, and ignored — the call still goes to Antigravity's own
/// prompt. So are `allowTool: true` and `permissionOverrides`, and so is a grant
/// written into `settings.json` from inside the hook, because permissions are
/// read once at session start. Only tightening works: `deny` blocks a call and
/// overrides an existing grant, and `force_ask` prompts even where a grant would
/// have let the call through.
///
/// Approve is therefore *silence*, not a decision — see
/// [`crate::agy_permissions`], which opens Antigravity's own gate ahead of time
/// so that saying nothing is the same as saying yes.
///
/// **The result is decoded with protojson.** Antigravity says so itself when it
/// rejects a malformed one:
///
/// ```text
/// failed to unmarshal result from hook jsonhook__agenttoast_PreToolUse_0_0
/// via protojson: duplicate field "permissionOverrides"
/// ```
///
/// protojson accepts *either* a field's proto name (`permission_overrides`) or
/// its `json_name` (`permissionOverrides`), and treats a message carrying both
/// as a duplicate field. That is worth knowing even though nothing here hedges
/// a spelling any more: it is why every field is written exactly once, in
/// camelCase, matching the convention Antigravity documents for its payloads.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgyHookResponse {
    /// `deny` or `force_ask`. Never `allow`, which Antigravity ignores, and
    /// never `ask`, which honours the allow list and so cannot be told apart
    /// from approving.
    pub decision: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl AgyHookResponse {
    /// Block the call. The reason reaches the model verbatim.
    fn deny(reason: &str) -> Self {
        Self {
            decision: "deny",
            reason: Some(reason.to_string()),
        }
    }

    /// Put the question to the user in their own terminal.
    ///
    /// `force_ask` rather than `ask` because `ask` honours the allow list —
    /// including the grants AgentToast writes to make Approve work, which would
    /// turn every deferral into an approval.
    fn force_ask(reason: Option<String>) -> Self {
        Self {
            decision: "force_ask",
            reason,
        }
    }
}

/// The buttons a permission toast carries.
///
/// `can_approve` is whether [`crate::agy_permissions::enabled`] holds — whether
/// Antigravity has been granted the tools the hook matches, so that a hook
/// saying nothing lets the call run. Without that, Approve cannot be honoured
/// by anything, and offering it would be a lie; "Approve in session" is then
/// the truthful version of the same button, and Deny works either way.
pub fn actions(can_approve: bool) -> Vec<Action> {
    let decide_elsewhere = Action {
        action_type: ActionType::OpenSession,
        label: if can_approve {
            "Open session".into()
        } else {
            "Approve in session".into()
        },
    };

    let mut actions = Vec::new();
    if can_approve {
        actions.push(Action {
            action_type: ActionType::Approve,
            label: "Approve".into(),
        });
    }
    actions.push(Action {
        action_type: ActionType::Deny,
        label: "Deny".into(),
    });
    actions.push(decide_elsewhere);
    actions
}

impl crate::AgentAdapter for AgyAdapter {
    fn agent_id(&self) -> &str {
        AGENT_ID
    }

    fn parse_hook_payload(&self, payload: &Value) -> Result<AttentionEvent> {
        let data: AgyToolCallPayload = serde_json::from_value(payload.clone())
            .context("Failed to parse Antigravity tool-call payload")?;

        let message = build_message(&data.tool_call.name, &data.tool_call.args);

        debug!(
            conversation_id = %data.conversation_id,
            tool = %data.tool_call.name,
            step = ?data.step_idx,
            "Parsed Antigravity hook payload"
        );

        let mut event = AttentionEvent::permission_request(
            &data.conversation_id,
            AGENT_ID,
            &message,
            Some(data.tool_call.name.clone()),
        );

        // Approve is offered only when it can do something. An Antigravity hook
        // cannot grant permission, so the button works by Antigravity having
        // granted the call already and this hook then staying quiet — which is
        // true only while AgentToast approval is switched on. With it off,
        // approving would look like an answer and leave the same prompt waiting
        // in the terminal, so the button that says so is the only one offered.
        event.actions = actions(crate::agy_permissions::approves(&data.tool_call.name));

        if let Some(args) = &data.tool_call.args {
            event.context = Some(serde_json::to_string_pretty(args).unwrap_or_default());
        }
        event.cwd = working_directory(&data);

        Ok(event)
    }

    fn format_response(&self, action: ActionType, text: Option<&str>) -> Result<String> {
        // Only reachable for PreToolUse; `format_for` routes everything else.
        match self.format_for(AgyHookKind::PreToolUse, action, text)? {
            Some(body) => Ok(body),
            None => Ok(String::new()),
        }
    }

    fn extract_session_id(&self, payload: &Value) -> Result<String> {
        payload
            .get("conversationId")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .context("Missing 'conversationId' in Antigravity hook payload")
    }
}

impl AgyAdapter {
    /// Format a user's answer for whichever hook asked the question.
    ///
    /// `None` means write nothing, which Antigravity reads as "this hook has no
    /// opinion" and resolves from its own permission lists. That is the whole of
    /// how Approve works — see [`crate::agy_permissions`] — and it is also the
    /// only correct answer for `Stop`, where any `decision` but `continue` is
    /// harmless and `continue` would refuse to let the agent stop.
    pub fn format_for(
        &self,
        kind: AgyHookKind,
        action: ActionType,
        text: Option<&str>,
    ) -> Result<Option<String>> {
        if kind != AgyHookKind::PreToolUse {
            return Ok(None);
        }

        let response = match action {
            // Nothing to say. The grants AgentToast wrote are what let the call
            // through; anything written here could only get in their way.
            ActionType::Approve | ActionType::Confirm => return Ok(None),
            ActionType::Deny | ActionType::Reject => {
                AgyHookResponse::deny("Denied by user via AgentToast")
            }
            ActionType::OpenSession => {
                AgyHookResponse::force_ask(Some("User opted to decide in the session".to_string()))
            }
            ActionType::SendText => {
                AgyHookResponse::force_ask(text.map(|t| format!("User replied: {}", t)))
            }
        };

        serde_json::to_string(&response)
            .map(Some)
            .context("Failed to serialize Antigravity hook response")
    }

    /// What the bridge writes when it never got an answer.
    ///
    /// A stopped daemon, an unreachable pipe, a toast nobody clicked. Silence
    /// would mean approval once the grants are in place, so the bridge says
    /// `force_ask` and Antigravity asks in its own terminal, exactly as it would
    /// if AgentToast were not installed.
    pub fn unanswered(reason: &str) -> String {
        serde_json::to_string(&AgyHookResponse::force_ask(Some(reason.to_string())))
            .unwrap_or_else(|_| r#"{"decision":"force_ask"}"#.to_string())
    }

    /// Turn a `Stop` payload into a "finished" toast.
    pub fn parse_stop(&self, payload: &Value) -> Result<Option<AttentionEvent>> {
        let data: AgyStop = serde_json::from_value(payload.clone())
            .context("Failed to parse Antigravity stop payload")?;

        let headline = match data.error.as_deref().map(str::trim).filter(|e| !e.is_empty()) {
            Some(error) => format!("Antigravity stopped: {}", shorten(error)),
            None => "Antigravity finished".to_string(),
        };

        let mut event = AttentionEvent::completed(&data.conversation_id, AGENT_ID, headline);
        event.cwd = data.workspace_paths.first().cloned();
        // Antigravity's Stop hook carries no closing message — unlike Claude
        // Code's, which hands over the last thing the model said. The reason
        // the loop ended is the only detail there is.
        event.context = data
            .termination_reason
            .filter(|r| !r.trim().is_empty())
            .map(|r| format!("Termination reason: {}", r));

        debug!(
            conversation_id = %data.conversation_id,
            reason = ?event.context,
            "Antigravity finished its turn"
        );

        Ok(Some(event))
    }
}

/// A `Stop` payload — the execution loop is ending.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgyStop {
    pub conversation_id: String,
    /// Why the loop ended: `NO_TOOL_CALL`, `max_steps_exceeded`, `error`, …
    #[serde(default)]
    pub termination_reason: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    /// False while background tasks are still running.
    #[serde(default)]
    pub fully_idle: bool,
    #[serde(default)]
    pub workspace_paths: Vec<String>,
}

/// Where the session is running.
///
/// `workspacePaths` is empty when the CLI is started outside a workspace, in
/// which case `run_command` still reports the directory it would run in.
fn working_directory(data: &AgyToolCallPayload) -> Option<String> {
    if let Some(path) = data.workspace_paths.first() {
        return Some(path.clone());
    }
    data.tool_call
        .args
        .as_ref()?
        .get("Cwd")?
        .as_str()
        .map(|s| s.to_string())
}

/// Longest headline worth putting on a toast before it stops being glanceable.
const HEADLINE_LIMIT: usize = 90;

fn build_message(tool_name: &str, args: &Option<Value>) -> String {
    // A question is the one case where the tool's own summary is the wrong
    // headline: "Ask about hooks" says nothing about what is being asked.
    if tool_name == "ask_question" {
        if let Some(question) = first_question(args) {
            return question;
        }
        return "Antigravity has a question".to_string();
    }

    // Antigravity writes a short label for every tool call it makes —
    // "Echo hookprobe" rather than the full shell line. That is better than
    // anything reconstructed from the raw arguments, and unlike Claude Code's
    // `description` it is present on every call rather than only on some.
    for field in ["toolSummary", "toolAction"] {
        if let Some(summary) = args
            .as_ref()
            .and_then(|a| a.get(field))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return shorten(summary);
        }
    }

    build_message_from_args(tool_name, args)
}

fn build_message_from_args(tool_name: &str, args: &Option<Value>) -> String {
    let field = |name: &str| {
        args.as_ref()
            .and_then(|a| a.get(name))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };

    match tool_name {
        "run_command" => match field("CommandLine") {
            Some(command) => format!("Antigravity wants to run: {}", shorten(command)),
            None => "Antigravity wants to run a command".to_string(),
        },
        "write_to_file" | "create_file" => {
            match field("TargetFile").or_else(|| field("AbsolutePath")) {
                Some(path) => format!("Antigravity wants to write to: {}", path),
                None => "Antigravity wants to write a file".to_string(),
            }
        }
        "replace_file_content" => match field("TargetFile").or_else(|| field("AbsolutePath")) {
            Some(path) => format!("Antigravity wants to edit: {}", path),
            None => "Antigravity wants to edit a file".to_string(),
        },
        _ => format!("Antigravity wants to use tool: {}", tool_name),
    }
}

/// The first question out of an `ask_question` payload.
///
/// Shaped as `{ "questions": [ { "question": "...", "options": [...] } ] }`,
/// with snake_case keys even though the surrounding payload is camelCase.
fn first_question(args: &Option<Value>) -> Option<String> {
    let text = args
        .as_ref()?
        .get("questions")?
        .as_array()?
        .first()?
        .get("question")?
        .as_str()?
        .trim();

    if text.is_empty() {
        None
    } else {
        Some(shorten(text))
    }
}

/// Shorten a line to fit a toast, breaking at a word boundary where possible.
fn shorten(line: &str) -> String {
    let line = line.trim();
    if line.chars().count() <= HEADLINE_LIMIT {
        return line.to_string();
    }

    let cut: String = line.chars().take(HEADLINE_LIMIT).collect();
    let cut = match cut.rsplit_once(' ') {
        Some((head, _)) if head.chars().count() > HEADLINE_LIMIT / 2 => head.to_string(),
        _ => cut,
    };
    format!("{}…", cut.trim_end_matches(['.', ',', ' ']))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AgentAdapter;
    use agenttoast_core::SessionState;
    use serde_json::json;

    /// A real payload, captured from `agy` running against a logging hook.
    fn run_command_payload() -> Value {
        json!({
            "artifactDirectoryPath": "C:/Users/x/.gemini/antigravity-cli/brain/380300d6",
            "conversationId": "380300d6-579e-4518-aaec-53facafbfec7",
            "modelName": "gemini-pro-agent",
            "stepIdx": 5,
            "toolCall": {
                "args": {
                    "CommandLine": "echo hookprobe",
                    "Cwd": "C:\\Users\\x\\.gemini\\antigravity-cli",
                    "WaitMsBeforeAsync": 2000,
                    "toolAction": "Running shell command",
                    "toolSummary": "Echo hookprobe"
                },
                "name": "run_command"
            },
            "transcriptPath": "C:/Users/x/.gemini/antigravity-cli/brain/380300d6/transcript.jsonl",
            "workspacePaths": []
        })
    }

    /// Antigravity payloads carry no event name, so shape is the only signal.
    #[test]
    fn events_are_classified_by_shape() {
        assert_eq!(
            AgyHookKind::from_payload(&run_command_payload()),
            AgyHookKind::PreToolUse
        );
        assert_eq!(
            AgyHookKind::from_payload(&json!({ "terminationReason": "NO_TOOL_CALL" })),
            AgyHookKind::Stop
        );
        assert_eq!(
            AgyHookKind::from_payload(&json!({ "stepIdx": 4 })),
            AgyHookKind::Unknown
        );
    }

    #[test]
    fn a_tool_call_becomes_a_permission_request() {
        let event = AgyAdapter.parse_hook_payload(&run_command_payload()).unwrap();

        assert_eq!(event.session_id, "380300d6-579e-4518-aaec-53facafbfec7");
        assert_eq!(event.agent, "agy");
        assert_eq!(event.tool_name.as_deref(), Some("run_command"));
        assert_eq!(event.state, SessionState::WaitingForPermission);
        // Antigravity's own label for the call, in preference to the raw line.
        assert_eq!(event.message, "Echo hookprobe");
        // The command itself is still there for the toast's detail line.
        assert!(event.context.unwrap().contains("echo hookprobe"));
    }

    /// Antigravity hooks cannot grant permission, so an Approve button would be
    /// a lie: it would answer the toast and leave the same prompt waiting in the
    /// terminal. Deny is real, and approving is the session's job.
    #[test]
    fn the_buttons_match_what_antigravity_will_honour() {
        // Approval on: a hook that says nothing lets the call run, so Approve
        // is real and the third button is only a way to go and look.
        let on: Vec<_> = actions(true).iter().map(|a| a.action_type.clone()).collect();
        assert_eq!(
            on,
            vec![
                ActionType::Approve,
                ActionType::Deny,
                ActionType::OpenSession
            ]
        );

        // Approval off: nothing would honour an Approve, so it is not offered,
        // and the remaining button says where approving actually happens.
        let off = actions(false);
        let kinds: Vec<_> = off.iter().map(|a| a.action_type.clone()).collect();
        assert!(
            !kinds.contains(&ActionType::Approve) && !kinds.contains(&ActionType::Confirm),
            "a toast must not offer an approval nothing will honour"
        );
        assert_eq!(kinds, vec![ActionType::Deny, ActionType::OpenSession]);
        assert_eq!(
            off.iter()
                .find(|a| a.action_type == ActionType::OpenSession)
                .unwrap()
                .label,
            "Approve in session"
        );
    }

    /// `workspacePaths` is empty when the CLI runs outside a workspace, which
    /// is exactly when "Open Session" most needs somewhere to point at.
    #[test]
    fn cwd_falls_back_to_the_commands_own_directory() {
        let event = AgyAdapter.parse_hook_payload(&run_command_payload()).unwrap();
        assert_eq!(
            event.cwd.as_deref(),
            Some("C:\\Users\\x\\.gemini\\antigravity-cli")
        );
    }

    #[test]
    fn a_workspace_wins_over_the_commands_directory() {
        let mut payload = run_command_payload();
        payload["workspacePaths"] = json!(["D:/work/project"]);

        let event = AgyAdapter.parse_hook_payload(&payload).unwrap();
        assert_eq!(event.cwd.as_deref(), Some("D:/work/project"));
    }

    #[test]
    fn without_a_summary_the_command_is_the_headline() {
        let mut payload = run_command_payload();
        payload["toolCall"]["args"] = json!({ "CommandLine": "npm run migrate" });

        let event = AgyAdapter.parse_hook_payload(&payload).unwrap();
        assert_eq!(event.message, "Antigravity wants to run: npm run migrate");
    }

    /// Captured from a real `ask_question` call. The summary Antigravity writes
    /// for it ("Ask about hooks") says nothing about what is being asked.
    #[test]
    fn a_question_becomes_the_headline() {
        let payload = json!({
            "conversationId": "c1",
            "toolCall": {
                "name": "ask_question",
                "args": {
                    "questions": [{
                        "is_multi_select": false,
                        "options": ["Yes", "No"],
                        "question": "Have you set up any custom rules or hooks?"
                    }],
                    "toolAction": "Asking about hooks",
                    "toolSummary": "Ask about hooks"
                }
            },
            "workspacePaths": []
        });

        let event = AgyAdapter.parse_hook_payload(&payload).unwrap();
        assert_eq!(event.message, "Have you set up any custom rules or hooks?");
    }

    #[test]
    fn a_question_without_text_still_reads_sensibly() {
        let payload = json!({
            "conversationId": "c1",
            "toolCall": { "name": "ask_question", "args": { "questions": [] } }
        });

        let event = AgyAdapter.parse_hook_payload(&payload).unwrap();
        assert_eq!(event.message, "Antigravity has a question");
    }

    fn decision(action: ActionType) -> Option<Value> {
        AgyAdapter
            .format_for(AgyHookKind::PreToolUse, action, None)
            .unwrap()
            .map(|raw| serde_json::from_str(&raw).unwrap())
    }

    /// Approve is silence. Antigravity gives a hook no way to grant a call —
    /// `decision: "allow"`, `allowTool: true` and `permissionOverrides` were
    /// each sent to a live session, parsed without complaint, and ignored — so
    /// approval is carried by the grants in `agy_permissions`, and this hook's
    /// job is to stay out of their way. Writing `allow` would be harmless but
    /// dishonest; writing anything else would block the call.
    #[test]
    fn approve_says_nothing() {
        assert!(
            decision(ActionType::Approve).is_none(),
            "approve must write nothing, so Antigravity resolves it from its own permissions"
        );
        assert!(decision(ActionType::Confirm).is_none());
    }

    /// The decision goes at the top level, not nested inside
    /// `hookSpecificOutput` the way Claude Code wants it. Emitting Claude's
    /// shape would leave the hook with no effect at all.
    #[test]
    fn a_decision_sits_at_the_top_level() {
        let out = decision(ActionType::Deny).expect("deny must answer");
        assert!(out.get("decision").is_some());
        assert!(
            out.get("hookSpecificOutput").is_none(),
            "Claude Code's shape must not leak into an Antigravity response"
        );
    }

    /// The result is unmarshalled with protojson, which accepts either a
    /// field's proto name or its `json_name` — and rejects a message carrying
    /// both:
    ///
    /// ```text
    /// failed to unmarshal result ... via protojson:
    /// duplicate field "permissionOverrides"
    /// ```
    ///
    /// A result that will not parse fails the tool call, so hedging a spelling
    /// is worse than picking one. Nothing hedges any more; this keeps it so.
    #[test]
    fn no_field_is_sent_under_two_spellings() {
        for action in [ActionType::Deny, ActionType::OpenSession, ActionType::SendText] {
            let out = decision(action.clone()).expect("must answer");
            let keys: Vec<&str> = out.as_object().unwrap().keys().map(String::as_str).collect();

            for (camel, snake) in [
                ("permissionOverrides", "permission_overrides"),
                ("allowTool", "allow_tool"),
                ("denyReason", "deny_reason"),
            ] {
                assert!(
                    !(keys.contains(&camel) && keys.contains(&snake)),
                    "{camel} and {snake} are one protojson field; sending both fails the parse"
                );
            }
        }
    }

    /// Nothing but `decision` and `reason` is sent. The other fields of
    /// `PreToolHookResult` were each tested and found to change nothing, and an
    /// unused field is one more way to trip the parser that gates every call.
    #[test]
    fn only_the_two_fields_that_do_something_are_sent() {
        let out = decision(ActionType::Deny).unwrap();
        let mut keys: Vec<&str> = out.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(keys, vec!["decision", "reason"]);
    }

    #[test]
    fn deny_blocks_with_a_reason() {
        let out = decision(ActionType::Deny).expect("deny must answer");
        assert_eq!(out["decision"], "deny");
        assert!(out["reason"].as_str().unwrap().contains("AgentToast"));
    }

    /// `ask` honours the "Always Allow" cache, so a user who chose to decide in
    /// the session could find the call already ran by the time they got there.
    #[test]
    fn open_session_forces_a_prompt() {
        assert_eq!(
            decision(ActionType::OpenSession).unwrap()["decision"],
            "force_ask"
        );
        assert_eq!(
            decision(ActionType::SendText).unwrap()["decision"],
            "force_ask"
        );
    }

    /// `ask` would be worse than useless here: it honours the allow list, and
    /// the allow list is exactly what AgentToast writes to make Approve work —
    /// so deferring with `ask` would approve every call it meant to hand back.
    /// Only `force_ask` overrides a grant.
    #[test]
    fn deferring_overrides_the_grant_rather_than_honouring_it() {
        for action in [ActionType::OpenSession, ActionType::SendText] {
            let out = decision(action.clone()).unwrap();
            assert_eq!(
                out["decision"], "force_ask",
                "{:?} must override the allow list, not defer to it",
                action
            );
        }
    }

    /// What the bridge writes when it never got an answer from the user.
    /// Antigravity ignores a hook that fails, and ignoring this one means
    /// approving the call, so failure has to be spelled out.
    #[test]
    fn an_unanswered_call_is_handed_back_not_approved() {
        let out: Value = serde_json::from_str(&AgyAdapter::unanswered("daemon is not running"))
            .expect("must be valid JSON, since unparseable output is ignored");
        assert_eq!(out["decision"], "force_ask");
        assert!(out["reason"].as_str().unwrap().contains("daemon"));
    }

    /// Antigravity validates this field and rejects anything else outright.
    #[test]
    fn every_action_uses_an_accepted_decision() {
        const ACCEPTED: [&str; 4] = ["allow", "deny", "ask", "force_ask"];

        for action in [
            ActionType::Approve,
            ActionType::Deny,
            ActionType::Confirm,
            ActionType::Reject,
            ActionType::SendText,
            ActionType::OpenSession,
        ] {
            // Approve answers with silence, which is itself a valid answer.
            let Some(out) = decision(action.clone()) else {
                continue;
            };
            let value = out["decision"].as_str().unwrap().to_string();
            assert!(
                ACCEPTED.contains(&value.as_str()),
                "{:?} produced unaccepted decision {:?}",
                action,
                value
            );
        }
    }

    /// `Stop` takes `{"decision": "continue"}` to mean "refuse to stop". Saying
    /// anything at all risks holding the agent open, and there is nothing
    /// waiting on the answer, so it must stay silent.
    #[test]
    fn stop_never_answers() {
        for kind in [AgyHookKind::Stop, AgyHookKind::Unknown] {
            let out = AgyAdapter
                .format_for(kind, ActionType::Approve, None)
                .unwrap();
            assert!(out.is_none(), "{:?} must not emit a decision", kind);
        }
    }

    /// Captured from a real Stop hook.
    #[test]
    fn finishing_raises_an_auto_dismissing_toast() {
        let event = AgyAdapter
            .parse_stop(&json!({
                "conversationId": "c1",
                "error": "",
                "executionNum": 0,
                "fullyIdle": true,
                "terminationReason": "NO_TOOL_CALL",
                "workspacePaths": ["D:/work/project"]
            }))
            .unwrap()
            .expect("finishing is worth a toast");

        assert_eq!(event.state, SessionState::Completed);
        assert_eq!(event.agent, "agy");
        assert_eq!(event.message, "Antigravity finished");
        assert_eq!(event.cwd.as_deref(), Some("D:/work/project"));
        assert!(event.context.unwrap().contains("NO_TOOL_CALL"));
    }

    #[test]
    fn stopping_on_an_error_says_so() {
        let event = AgyAdapter
            .parse_stop(&json!({
                "conversationId": "c1",
                "error": "exit status 1",
                "terminationReason": "error"
            }))
            .unwrap()
            .unwrap();

        assert_eq!(event.message, "Antigravity stopped: exit status 1");
    }

    #[test]
    fn the_session_id_is_the_conversation_id() {
        let id = AgyAdapter.extract_session_id(&run_command_payload()).unwrap();
        assert_eq!(id, "380300d6-579e-4518-aaec-53facafbfec7");
    }

    #[test]
    fn a_long_headline_is_shortened_at_a_word_boundary() {
        let long = "a".repeat(40) + " " + &"b".repeat(80);
        let mut payload = run_command_payload();
        payload["toolCall"]["args"] = json!({ "CommandLine": long });

        let event = AgyAdapter.parse_hook_payload(&payload).unwrap();
        assert!(event.message.ends_with('…'));
    }

    /// The default matcher decides what the user actually sees. Reading tools
    /// firing a toast on every step is the failure mode to avoid.
    #[test]
    fn the_default_matcher_covers_writes_and_not_reads() {
        for tool in [
            "run_command",
            "write_to_file",
            "create_file",
            "replace_file_content",
        ] {
            assert!(matcher_covers(tool), "{tool} should raise a toast");
        }
        for tool in [
            "view_file",
            "list_dir",
            "find_by_name",
            "grep_search",
            "codebase_search",
        ] {
            assert!(!matcher_covers(tool), "{tool} should stay silent");
        }
    }

    /// The matcher is a regex Antigravity compiles, not something this crate
    /// evaluates; this checks the alternation by hand so the constant cannot
    /// quietly lose a tool.
    fn matcher_covers(tool: &str) -> bool {
        DEFAULT_MATCHER
            .trim_start_matches("^(")
            .trim_end_matches(")$")
            .split('|')
            .any(|candidate| candidate == tool)
    }
}
