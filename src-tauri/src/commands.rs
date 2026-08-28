//! Tauri commands — invokable from the frontend JavaScript.

use crate::AppState;
use agenttoast_adapters::agy::Watch;
use agenttoast_core::event::{ActionType, AttentionEvent, UserResponse};
use agenttoast_core::session::Session;
use std::sync::Arc;
use uuid::Uuid;

/// Get all active sessions.
#[tauri::command]
pub async fn get_sessions(state: tauri::State<'_, Arc<AppState>>) -> Result<Vec<Session>, String> {
    Ok(state.sessions.all().await)
}

/// Get the one attention event a toast window is responsible for.
///
/// Each toast window carries its event id in its label, so it asks for its own
/// event by id rather than guessing at the first pending one — otherwise two
/// concurrent toasts both render whichever event happens to come back first.
#[tauri::command]
pub async fn get_event(
    state: tauri::State<'_, Arc<AppState>>,
    event_id: String,
) -> Result<Option<AttentionEvent>, String> {
    let event_uuid = Uuid::parse_str(&event_id).map_err(|e| format!("Invalid event ID: {}", e))?;

    Ok(state
        .sessions
        .all()
        .await
        .into_iter()
        .filter_map(|s| s.attention_request)
        .find(|e| e.event_id == event_uuid))
}

/// Reveal a toast once its frontend has painted and measured itself.
#[tauri::command]
pub fn toast_ready(app: tauri::AppHandle, event_id: String, height: f64) {
    crate::window::mark_ready(&app, &event_id, height);
}

/// Get all sessions with pending attention events.
#[tauri::command]
pub async fn get_pending_events(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<Session>, String> {
    Ok(state.sessions.attention_needed().await)
}

/// Respond to an attention event (approve, deny, etc.).
#[tauri::command]
pub async fn respond_to_event(
    state: tauri::State<'_, Arc<AppState>>,
    event_id: String,
    action: String,
    text_input: Option<String>,
) -> Result<(), String> {
    let event_uuid = Uuid::parse_str(&event_id).map_err(|e| format!("Invalid event ID: {}", e))?;

    let action_type = match action.as_str() {
        "approve" => ActionType::Approve,
        "deny" => ActionType::Deny,
        "confirm" => ActionType::Confirm,
        "reject" => ActionType::Reject,
        "send_text" => ActionType::SendText,
        "open_session" => ActionType::OpenSession,
        _ => return Err(format!("Unknown action: {}", action)),
    };

    // "Open session" means take me to the agent, so raise its terminal before
    // handing the decision back. Do this first: resolving the event unblocks
    // the bridge, which clears the session's pending request.
    if action_type == ActionType::OpenSession {
        match pending_event(&state, event_uuid).await {
            Some(event) => {
                crate::focus::focus_agent_window(event.process_id.unwrap_or_default());
            }
            None => {
                tracing::warn!(
                    event_id = %event_id,
                    "Request is no longer pending; nothing to raise"
                );
            }
        }
    }

    let response = UserResponse {
        event_id: event_uuid,
        action: action_type,
        text_input,
    };

    // A notification toast has no pending request behind it, and an event
    // answered from the dashboard is already gone by the time its toast is
    // clicked. Neither is an error — the toast simply closes.
    if let Err(e) = state.router.resolve(response).await {
        tracing::debug!(event_id = %event_id, reason = %e, "Nothing was waiting on this answer");
    }

    Ok(())
}

/// Dismiss an event without responding (timeout or user closed toast).
#[tauri::command]
pub async fn dismiss_event(
    state: tauri::State<'_, Arc<AppState>>,
    event_id: String,
) -> Result<(), String> {
    let event_uuid = Uuid::parse_str(&event_id).map_err(|e| format!("Invalid event ID: {}", e))?;
    state.router.cancel(&event_uuid).await;
    Ok(())
}

/// Take a toast off screen without answering it.
///
/// Closing a toast is "not now", not "decide without me": the request stays
/// pending and the agent stays blocked until it is answered here or from the
/// dashboard.
#[tauri::command]
pub fn hide_toast(app: tauri::AppHandle, event_id: String) {
    crate::window::hide_toast(&app, &event_id);
}

/// Close the toast window from the frontend.
#[tauri::command]
pub fn close_window(
    app: tauri::AppHandle,
    event_id: String,
) -> Result<(), String> {
    crate::window::close_toast(&app, &event_id).map_err(|e| e.to_string())
}

/// Re-open the toast for a request that was hidden.
///
/// Hiding a toast leaves the request pending and the agent blocked, so there
/// has to be a way to pull it back without answering from the dashboard.
#[tauri::command]
pub async fn reopen_toast(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    event_id: String,
) -> Result<(), String> {
    let event_uuid = Uuid::parse_str(&event_id).map_err(|e| format!("Invalid event ID: {}", e))?;

    let event = state
        .sessions
        .all()
        .await
        .into_iter()
        .filter_map(|s| s.attention_request)
        .find(|e| e.event_id == event_uuid)
        .ok_or_else(|| "That request is no longer pending".to_string())?;

    crate::window::restore(&app, &event);
    Ok(())
}

/// Where the Claude Code bridge lives, if the app can find it.
///
/// Returned as a string so the dashboard can show it, and so the hook config
/// can be written with an absolute path.
#[tauri::command]
pub fn bridge_path(app: tauri::AppHandle) -> Option<String> {
    crate::install::bridge_path(&app).map(|p| p.display().to_string())
}

/// The still-pending attention event with this id, if any.
async fn pending_event(state: &Arc<AppState>, event_id: Uuid) -> Option<AttentionEvent> {
    state
        .sessions
        .all()
        .await
        .into_iter()
        .filter_map(|s| s.attention_request)
        .find(|e| e.event_id == event_id)
}

/* ------------------------------------------------------- Claude Code setup --- */

/// Resolve a scope name from the frontend into a real one.
fn scope_from(scope: &str, project: Option<String>) -> Result<crate::hooks::Scope, String> {
    match scope {
        "global" => Ok(crate::hooks::Scope::Global),
        "project" => project
            .map(|p| crate::hooks::Scope::Project(std::path::PathBuf::from(p)))
            .ok_or_else(|| "No project folder was given".to_string()),
        other => Err(format!("Unknown scope: {}", other)),
    }
}

/// Start showing a row for this project, whether or not it is connected yet.
///
/// Adding and connecting are separate acts: a project may already be wired up
/// from an earlier session, in which case the user never presses Connect and
/// nothing would ever record it.
#[tauri::command]
pub fn add_project(project: String) {
    crate::hooks::remember_project(&project);
}

/// Stop showing a row for this project. Leaves its hooks alone.
#[tauri::command]
pub fn remove_project(project: String) {
    crate::hooks::forget_project(&project);
}

/// Whether Claude Code is currently wired up, globally and for a project.
#[tauri::command]
pub fn hook_status(
    app: tauri::AppHandle,
    project: Option<String>,
) -> Result<Vec<crate::hooks::HookStatus>, String> {
    let bridge = crate::install::bridge_path(&app);
    let bridge = bridge.as_deref();

    let mut out = vec![crate::hooks::status(&crate::hooks::Scope::Global, bridge)];

    let mut projects = crate::hooks::remembered_projects();
    if let Some(dir) = project {
        if !projects.iter().any(|p| p.eq_ignore_ascii_case(&dir)) {
            projects.push(dir);
        }
    }

    for dir in projects {
        out.push(crate::hooks::status(
            &crate::hooks::Scope::Project(std::path::PathBuf::from(dir)),
            bridge,
        ));
    }
    Ok(out)
}

/// Add AgentToast's hooks to Claude Code's settings.
#[tauri::command]
pub fn connect_hooks(
    app: tauri::AppHandle,
    scope: String,
    project: Option<String>,
) -> Result<crate::hooks::HookStatus, String> {
    let bridge = crate::install::bridge_path(&app).ok_or_else(|| {
        "Could not find the AgentToast bridge. If this is a development build, \
         run `cargo build --workspace` first."
            .to_string()
    })?;

    let scope = scope_from(&scope, project)?;
    let status = crate::hooks::connect(&scope, &bridge)?;

    // Keep a row for it after the dashboard is closed and reopened.
    if let Some(dir) = &status.project {
        crate::hooks::remember_project(dir);
    }

    tracing::info!(path = %status.path, bridge = %bridge.display(), "Connected Claude Code");
    Ok(status)
}

/// Remove AgentToast's hooks, leaving everything else in the file untouched.
#[tauri::command]
pub fn disconnect_hooks(
    scope: String,
    project: Option<String>,
) -> Result<crate::hooks::HookStatus, String> {
    let scope = scope_from(&scope, project)?;
    let status = crate::hooks::disconnect(&scope)?;

    tracing::info!(path = %status.path, "Disconnected Claude Code");
    Ok(status)
}

/* -------------------------------------------------------- Antigravity setup --- */

/// Where the Antigravity bridge lives, if the app can find it.
#[tauri::command]
pub fn agy_bridge_path(app: tauri::AppHandle) -> Option<String> {
    crate::install::agy_bridge_path(&app).map(|p| p.display().to_string())
}

/// Whether Antigravity is currently wired up.
///
/// One row rather than a list: Antigravity's other hooks location is a
/// workspace's `.agents/hooks.json`, which is meant to be shared through
/// version control, so AgentToast only ever writes the global one.
#[tauri::command]
pub fn agy_hook_status(app: tauri::AppHandle) -> crate::hooks::HookStatus {
    let bridge = crate::install::agy_bridge_path(&app);
    crate::agy_hooks::status(bridge.as_deref())
}

/// Add AgentToast's hooks to Antigravity's `hooks.json`.
///
/// `watch_file_edits` decides the matcher. Antigravity gates file writes through
/// its *execution mode* rather than its permission lists, and a hook cannot see
/// the mode — so an `accept-edits` session, which was chosen precisely to stop
/// pausing on edits, needs a way to say "commands only". See
/// `agenttoast_adapters::agy::Watch`.
#[tauri::command]
pub fn connect_agy_hooks(
    app: tauri::AppHandle,
    watch_file_edits: bool,
) -> Result<crate::hooks::HookStatus, String> {
    let bridge = crate::install::agy_bridge_path(&app).ok_or_else(|| {
        "Could not find the AgentToast bridge for Antigravity. If this is a \
         development build, run `cargo build --workspace` first."
            .to_string()
    })?;

    let watch = if watch_file_edits {
        Watch::CommandsAndEdits
    } else {
        Watch::CommandsOnly
    };
    let status = crate::agy_hooks::connect(&bridge, watch)?;

    // The grants have to follow the matcher. Narrowing the scope while
    // `write_file(*)` stayed behind would leave Antigravity granted for edits
    // that nothing is watching any more.
    if agenttoast_adapters::agy_permissions::enabled_for(Watch::CommandsOnly) {
        if let Err(e) = agenttoast_adapters::agy_permissions::enable(watch) {
            tracing::warn!(error = %e, "Could not realign Antigravity approval grants");
        }
    }

    tracing::info!(
        path = %status.path,
        bridge = %bridge.display(),
        ?watch,
        "Connected Antigravity"
    );
    Ok(status)
}

/// Remove AgentToast's hooks, leaving every other named hook untouched.
///
/// Also gives back the approval grants. They are only safe while the bridge is
/// there to answer for them: on their own they are a standing instruction to
/// Antigravity to stop asking, which is the last thing a disconnected AgentToast
/// should leave behind.
#[tauri::command]
pub fn disconnect_agy_hooks() -> Result<crate::hooks::HookStatus, String> {
    if let Err(e) = agenttoast_adapters::agy_permissions::disable() {
        tracing::warn!(error = %e, "Could not withdraw Antigravity approval grants");
    }

    let status = crate::agy_hooks::disconnect()?;

    tracing::info!(path = %status.path, "Disconnected Antigravity");
    Ok(status)
}

/// What Approve can currently do on an Antigravity toast.
#[derive(serde::Serialize)]
pub struct AgyApprovalStatus {
    /// Which calls the configured matcher raises a toast for.
    pub watches_file_edits: bool,
    /// Antigravity's startup execution mode, if it is written down. Advisory:
    /// `--mode` and Shift+Tab change the live mode without recording it.
    pub agent_mode: Option<String>,
    /// Whether every grant is in place. Approve is offered only when it is.
    pub enabled: bool,
    /// `~/.gemini/antigravity-cli/settings.json`, for the dashboard to name.
    pub path: String,
    /// The rules that get written, so the user can see the size of it.
    pub grants: Vec<String>,
    /// Rules of the user's own that would beat ours. Antigravity resolves
    /// deny > ask > allow, so an existing `ask(command(*))` keeps prompting no
    /// matter what we write, and the user deserves to be told rather than left
    /// wondering why Approve does nothing.
    pub shadowed_by: Vec<String>,
}

fn approval_status() -> AgyApprovalStatus {
    use agenttoast_adapters::agy_permissions as perms;

    // Read the scope off the hooks file rather than storing it twice. The
    // matcher Antigravity will actually run is the only thing that decides which
    // calls raise a toast, so it is the honest source for the answer.
    let watch = crate::agy_hooks::configured_watch();

    AgyApprovalStatus {
        watches_file_edits: watch.watches_edits(),
        agent_mode: perms::agent_mode(),
        enabled: perms::enabled_for(watch),
        path: perms::settings_path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<unknown>".into()),
        grants: perms::grants_for(watch)
            .iter()
            .map(|g| g.to_string())
            .collect(),
        shadowed_by: perms::shadowing().unwrap_or_default(),
    }
}

/// Read the approval state without changing anything.
#[tauri::command]
pub fn agy_approval_status() -> AgyApprovalStatus {
    approval_status()
}

/// Let AgentToast answer Antigravity's permission questions.
///
/// Writes the allow-rules that make a silent hook mean "yes". Nothing here
/// touches Antigravity's `deny` or `ask` lists, and an existing rule of the
/// user's is left in place and reported back rather than overwritten.
#[tauri::command]
pub fn enable_agy_approval() -> Result<AgyApprovalStatus, String> {
    let watch = crate::agy_hooks::configured_watch();
    let shadowed =
        agenttoast_adapters::agy_permissions::enable(watch).map_err(|e| e.to_string())?;

    if !shadowed.is_empty() {
        tracing::warn!(rules = ?shadowed, "Antigravity rules take precedence over the grants");
    }
    tracing::info!(?watch, "Antigravity approval enabled");
    Ok(approval_status())
}

/// Take the grants back, returning Antigravity to asking for itself.
#[tauri::command]
pub fn disable_agy_approval() -> Result<AgyApprovalStatus, String> {
    agenttoast_adapters::agy_permissions::disable().map_err(|e| e.to_string())?;

    tracing::info!("Antigravity approval disabled");
    Ok(approval_status())
}
