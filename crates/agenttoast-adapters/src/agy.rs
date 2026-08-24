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
/// Flat, unlike Claude Code's nested `hookSpecificOutput` — and carrying two
/// different answers to the same question, because Antigravity's
/// `PreToolHookResult` message holds both interfaces at once:
///
/// ```text
/// decision, reason, overwrite, permission_overrides   <- the "full" interface
/// allow_tool, deny_reason                             <- the older pair
/// ```
///
/// A hook answering `{"decision":"allow"}` was observed being parsed without
/// complaint and then ignored, with Antigravity going on to raise its own
/// confirmation prompt anyway. Adding `allow_tool: true` changed nothing.
///
/// **The result is decoded with protojson.** Antigravity says so itself when it
/// rejects a malformed one:
///
/// ```text
/// failed to unmarshal result from hook jsonhook__agenttoast_PreToolUse_0_0
/// via protojson: duplicate field "permissionOverrides"
/// ```
///
/// That settles the spelling question, and warns against hedging it. protojson
/// accepts *either* the proto field name (`permission_overrides`) or its
/// `json_name` (`permissionOverrides`) — but sending both is a **duplicate
/// field**, which fails the parse, and a hook whose result will not parse fails
/// the tool call outright. So each field is written exactly once, in camelCase,
/// matching the convention Antigravity documents for its own payloads.
///
/// **None of these can approve a call**, and that is Antigravity's design rather
/// than a spelling problem here. It defines `hook_deny`, `hook_force_ask` and
/// `hook_deny_unless_prior_grant`, and no `hook_allow`: a hook can tighten
/// permissions or force a prompt, never loosen one. `decision: "allow"`,
/// `allowTool: true` and `permissionOverrides` were each tested against a live
/// session, parsed cleanly, and ignored — while `deny` works and even overrides
/// an existing grant. Writing the grant into `settings.json` from inside the
/// hook does not help either, because permissions are read once at session
/// start.
///
/// They are still sent. They cost nothing, they say plainly what the user chose,
/// and they become correct the day Antigravity grows an allow path. What they do
/// *not* do is spare the user its confirmation prompt. See `TODO.md`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgyHookResponse {
    /// `allow`, `deny`, `ask`, or `force_ask`.
    pub decision: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The older interface's verdict. Omitted rather than set to `false` when
    /// handing the decision back: absent means "no opinion" and lets
    /// Antigravity ask, where `false` would deny outright.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_tool: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deny_reason: Option<String>,
    /// Temporary permission grants — what actually lets the call through.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub permission_overrides: Vec<String>,
}

impl AgyHookResponse {
    /// Allow the call, in every dialect, carrying any grant that goes with it.
    fn allow(reason: &str, grants: Vec<String>) -> Self {
        Self {
            decision: "allow",
            reason: Some(reason.to_string()),
            allow_tool: Some(true),
            deny_reason: None,
            permission_overrides: grants,
        }
    }

    /// Block the call, in both dialects.
    fn deny(reason: &str) -> Self {
        Self {
            decision: "deny",
            reason: Some(reason.to_string()),
            allow_tool: Some(false),
            deny_reason: Some(reason.to_string()),
            permission_overrides: Vec::new(),
        }
    }

    /// Say nothing decisive and let Antigravity put the question to the user.
    ///
    /// `allow_tool` is left out on purpose: this is the one case where the two
    /// interfaces cannot agree, since the older pair has no way to express
    /// "ask" and `false` would mean deny. No grant either — granting is the
    /// opposite of handing the decision back.
    fn defer(decision: &'static str, reason: Option<String>) -> Self {
        Self {
            decision,
            reason,
            allow_tool: None,
            deny_reason: None,
            permission_overrides: Vec::new(),
        }
    }
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

        // No Approve button, because there is nothing behind it. Antigravity
        // hooks cannot grant permission — see the note on `AgyHookResponse` —
        // so an Approve here would look like it answered the question and then
        // leave the same prompt waiting in the terminal. Deny genuinely blocks
        // the call; approving is something only the session can do, which is
        // what the remaining button says.
        event.actions = vec![
            Action {
                action_type: ActionType::Deny,
                label: "Deny".into(),
            },
            Action {
                action_type: ActionType::OpenSession,
                label: "Approve in session".into(),
            },
        ];

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
    /// `None` means write nothing. Antigravity reads an empty stdout as "this
    /// hook has no opinion" and falls back to its own permission flow, which is
    /// the only safe answer when there is nothing to say — and the only correct
    /// one for `Stop`, where any `decision` other than `continue` is fine but
    /// `continue` would refuse to let the agent stop.
    pub fn format_for(
        &self,
        kind: AgyHookKind,
        action: ActionType,
        text: Option<&str>,
    ) -> Result<Option<String>> {
        self.format_for_call(kind, action, text, Vec::new())
    }

    /// As [`Self::format_for`], but carrying the permission grants that let an
    /// approved call actually run.
    ///
    /// Kept separate because the grants have to be derived from the tool call,
    /// which the plain signature does not have.
    pub fn format_for_call(
        &self,
        kind: AgyHookKind,
        action: ActionType,
        text: Option<&str>,
        grants: Vec<String>,
    ) -> Result<Option<String>> {
        if kind != AgyHookKind::PreToolUse {
            return Ok(None);
        }

        let response = match action {
            ActionType::Approve | ActionType::Confirm => {
                AgyHookResponse::allow("Approved by user via AgentToast", grants)
            }
            ActionType::Deny | ActionType::Reject => {
                AgyHookResponse::deny("Denied by user via AgentToast")
            }
            // The user went to the session to decide there. `force_ask` rather
            // than `ask` because `ask` honours the "Always Allow" cache: with a
            // cached grant the call would simply run, and they would arrive to
            // find the decision already made for them.
            ActionType::OpenSession => AgyHookResponse::defer(
                "force_ask",
                Some("User opted to decide in the session".to_string()),
            ),
            ActionType::SendText => AgyHookResponse::defer(
                "force_ask",
                text.map(|t| format!("User replied: {}", t)),
            ),
        };

        serde_json::to_string(&response)
            .map(Some)
            .context("Failed to serialize Antigravity hook response")
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

/// The permission grants that let this tool call run without a prompt.
///
/// Antigravity's own rule vocabulary, the same one used by `permissions.allow`
/// in `settings.json` and named in its headless error message ("Add an
/// allow-rule under permissions.allow (e.g. `command(<target>)`)"):
///
/// - `command(<command line>)` for shell calls. Matched as a prefix on word
///   boundaries — the docs' example is that `git commit` matches
///   `git commit -m "msg"` but not `git commit-next` — so passing the exact
///   command line grants that call and no more.
/// - `write_file(<path>)` for anything that writes. Applies to files and
///   directories alike, recursively.
///
/// An unrecognised tool yields nothing rather than a guessed rule: a wrong
/// grant would be a silent over-permission, and the worst case without one is
/// the prompt the user already sees today.
pub fn permission_grants(payload: &Value) -> Vec<String> {
    let Some(call) = payload.get("toolCall") else {
        return Vec::new();
    };
    let name = call.get("name").and_then(|n| n.as_str()).unwrap_or_default();
    let arg = |key: &str| {
        call.get("args")
            .and_then(|a| a.get(key))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };

    let grant = match name {
        "run_command" => arg("CommandLine").map(|c| format!("command({})", c)),
        "write_to_file" | "create_file" | "replace_file_content" | "edit_notebook"
        | "delete_file" => arg("TargetFile")
            .or_else(|| arg("AbsolutePath"))
            .map(|p| format!("write_file({})", p)),
        _ => None,
    };

    grant.into_iter().collect()
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
    fn there_is_no_approve_button() {
        let event = AgyAdapter.parse_hook_payload(&run_command_payload()).unwrap();
        let kinds: Vec<_> = event.actions.iter().map(|a| a.action_type.clone()).collect();

        assert!(
            !kinds.contains(&ActionType::Approve) && !kinds.contains(&ActionType::Confirm),
            "an Antigravity toast must not offer an approval it cannot deliver"
        );
        assert_eq!(kinds, vec![ActionType::Deny, ActionType::OpenSession]);

        // The remaining button has to say how approving actually happens.
        let open = event
            .actions
            .iter()
            .find(|a| a.action_type == ActionType::OpenSession)
            .unwrap();
        assert_eq!(open.label, "Approve in session");
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

    /// The decision is top level here, not nested inside `hookSpecificOutput`
    /// the way Claude Code wants it. Emitting Claude's shape would leave the
    /// hook with no effect at all.
    #[test]
    fn approve_allows_at_the_top_level() {
        let out = decision(ActionType::Approve).expect("approve must answer");
        assert_eq!(out["decision"], "allow");
        assert!(
            out.get("hookSpecificOutput").is_none(),
            "Claude Code's shape must not leak into an Antigravity response"
        );
    }

    /// `PreToolHookResult` carries two interfaces at once, so the older
    /// verdict goes out alongside the newer one.
    #[test]
    fn approve_also_answers_the_older_interface() {
        let out = decision(ActionType::Approve).unwrap();
        assert_eq!(out["allowTool"], true);
    }

    /// The result is unmarshalled with protojson, which accepts both a field's
    /// proto name and its `json_name` — and rejects a message carrying both:
    ///
    /// ```text
    /// failed to unmarshal result ... via protojson:
    /// duplicate field "permissionOverrides"
    /// ```
    ///
    /// A result that will not parse fails the tool call, so hedging the
    /// spelling is worse than picking one. Every key must appear exactly once.
    #[test]
    fn no_field_is_sent_under_two_spellings() {
        let raw = AgyAdapter
            .format_for_call(
                AgyHookKind::PreToolUse,
                ActionType::Approve,
                None,
                vec!["command(echo hi)".to_string()],
            )
            .unwrap()
            .unwrap();
        let out: Value = serde_json::from_str(&raw).unwrap();

        for (camel, snake) in [
            ("permissionOverrides", "permission_overrides"),
            ("allowTool", "allow_tool"),
            ("denyReason", "deny_reason"),
        ] {
            assert!(
                !(out.get(camel).is_some() && out.get(snake).is_some()),
                "{camel} and {snake} are one protojson field; sending both fails the parse"
            );
        }
    }

    /// The binary defines `hook_deny` and `hook_force_ask` but no `hook_allow`:
    /// a hook cannot grant a call, only block or escalate it. Approval has to
    /// travel as a permission grant, under both spellings of the field.
    #[test]
    fn approve_grants_permission_for_the_call() {
        let grants = permission_grants(&run_command_payload());
        assert_eq!(grants, vec!["command(echo hookprobe)".to_string()]);

        let raw = AgyAdapter
            .format_for_call(AgyHookKind::PreToolUse, ActionType::Approve, None, grants)
            .unwrap()
            .expect("approve must answer");
        let out: Value = serde_json::from_str(&raw).unwrap();

        assert_eq!(out["permissionOverrides"][0], "command(echo hookprobe)");
    }

    /// A write is granted by path, not by command. `write_file` covers files
    /// and directories alike.
    #[test]
    fn writes_are_granted_by_path() {
        let payload = json!({
            "conversationId": "c1",
            "toolCall": {
                "name": "write_to_file",
                "args": { "TargetFile": "C:/Users/x/toast-test/hello.py" }
            }
        });

        assert_eq!(
            permission_grants(&payload),
            vec!["write_file(C:/Users/x/toast-test/hello.py)".to_string()]
        );
    }

    /// A guessed rule would be a silent over-permission; the cost of none is
    /// only the prompt the user already gets.
    #[test]
    fn an_unknown_tool_grants_nothing() {
        let payload = json!({
            "conversationId": "c1",
            "toolCall": { "name": "browser_click_element", "args": { "Index": 3 } }
        });
        assert!(permission_grants(&payload).is_empty());
    }

    /// Denying and deferring must never hand out a grant.
    #[test]
    fn only_approval_grants() {
        let grants = permission_grants(&run_command_payload());

        for action in [ActionType::Deny, ActionType::OpenSession, ActionType::SendText] {
            let raw = AgyAdapter
                .format_for_call(
                    AgyHookKind::PreToolUse,
                    action.clone(),
                    None,
                    grants.clone(),
                )
                .unwrap()
                .expect("must answer");
            let out: Value = serde_json::from_str(&raw).unwrap();
            assert!(
                out.get("permissionOverrides").is_none(),
                "{:?} must not grant permission",
                action
            );
        }
    }

    #[test]
    fn deny_blocks_with_a_reason() {
        let out = decision(ActionType::Deny).expect("deny must answer");
        assert_eq!(out["decision"], "deny");
        assert!(out["reason"].as_str().unwrap().contains("AgentToast"));
        assert_eq!(out["allowTool"], false);
        assert!(out["denyReason"].as_str().unwrap().contains("AgentToast"));
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

    /// The older interface has no way to say "ask", and `false` there means
    /// deny — so handing the decision back must leave the field out entirely
    /// rather than answering on the user's behalf.
    #[test]
    fn deferring_omits_the_older_verdict() {
        for action in [ActionType::OpenSession, ActionType::SendText] {
            let out = decision(action.clone()).unwrap();
            assert!(
                out.get("allowTool").is_none(),
                "{:?} must not emit a verdict in the older interface",
                action
            );
        }
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
            let out = decision(action.clone()).expect("every action must answer");
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
