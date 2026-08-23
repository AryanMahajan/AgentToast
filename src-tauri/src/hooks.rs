//! Wiring AgentToast into Claude Code's settings file.
//!
//! Installing AgentToast does not connect it to anything: Claude Code only
//! knows about the bridge because a `hooks` block in its settings tells it to
//! run one. That block used to be the user's job, which meant hand-editing JSON
//! to wire up something security-sensitive — the easiest possible way to break
//! a working Claude Code install.
//!
//! Two scopes are supported, because both are legitimate: connect globally and
//! every project gets toasts, or connect one project and nothing else changes.
//!
//! Everything here treats the settings file as somebody else's: the existing
//! contents are preserved, key order is kept, a backup is written before any
//! change, and our own entries are recognised so connecting twice updates them
//! rather than stacking duplicates.

use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

/// Hook events AgentToast registers for.
///
/// `PermissionRequest` rather than `PreToolUse`, because it fires only once
/// Claude Code has decided it needs a human. `PreToolUse` fires for every tool
/// call regardless, which means it overrides the permission mode the user
/// chose: in `auto` or `acceptEdits` they have explicitly said "stop asking",
/// and a toast on every call ignores that.
const PERMISSION_REQUEST: &str = "PermissionRequest";
const SESSION_START: &str = "SessionStart";
const SESSION_END: &str = "SessionEnd";

/// Any tool the user is being asked about is worth a toast, so no matcher is
/// set: by the time this event fires, Claude Code has already decided it needs
/// an answer.
const NO_MATCHER: Option<&str> = None;

/// Left behind by earlier versions, which hooked PreToolUse. Recognised so
/// connecting cleans it up rather than leaving both wired at once.
const SUPERSEDED_EVENTS: [&str; 1] = ["PreToolUse"];

/// Only start a session on a real start or resume, not on every clear/compact.
const SESSION_START_MATCHER: &str = "startup|resume";

/// Identifies our own hook entries whatever path they point at, so a moved or
/// reinstalled AgentToast is recognised and refreshed instead of duplicated.
const BRIDGE_MARKER: &str = "agenttoast-bridge-claude";

/// Where a set of hooks lives.
#[derive(Debug, Clone)]
pub enum Scope {
    /// `~/.claude/settings.json` — applies to every project.
    Global,
    /// `<project>/.claude/settings.json` — applies to one project only.
    Project(PathBuf),
}

impl Scope {
    /// The project folder this scope refers to, or `None` when global.
    pub fn project_path(&self) -> Option<String> {
        match self {
            Scope::Global => None,
            Scope::Project(dir) => Some(dir.display().to_string()),
        }
    }

    /// The settings file this scope writes to.
    pub fn settings_path(&self) -> Option<PathBuf> {
        match self {
            Scope::Global => Some(dirs::home_dir()?.join(".claude").join("settings.json")),
            Scope::Project(dir) => Some(dir.join(".claude").join("settings.json")),
        }
    }
}

/// What the settings file currently says about AgentToast.
#[derive(Debug, serde::Serialize)]
pub struct HookStatus {
    /// The project this row is for, or `None` for the global scope. Lets the
    /// dashboard act on a row without having to remember what it asked for.
    pub project: Option<String>,
    /// The settings file this describes.
    pub path: String,
    /// Whether the file exists at all.
    pub exists: bool,
    /// Whether it references our bridge.
    pub connected: bool,
    /// The bridge path it points at, if connected.
    pub bridge: Option<String>,
    /// Connected, but to a bridge that is not the one this app would use —
    /// left behind by a move or an earlier install, and silently broken.
    pub stale: bool,
}

/// Read the current state without changing anything.
pub fn status(scope: &Scope, current_bridge: Option<&Path>) -> HookStatus {
    let Some(path) = scope.settings_path() else {
        return HookStatus {
            project: scope.project_path(),
            path: "<unknown>".into(),
            exists: false,
            connected: false,
            bridge: None,
            stale: false,
        };
    };

    let display = path.display().to_string();
    let Some(settings) = read_settings(&path) else {
        return HookStatus {
            project: scope.project_path(),
            path: display,
            exists: path.exists(),
            connected: false,
            bridge: None,
            stale: false,
        };
    };

    let bridge = find_bridge_command(&settings);

    // Deliberately an exact comparison against what would be written, not a
    // "do these point at the same file" test. A command can name the right
    // binary and still be unusable — a `\\?\` prefix or backslash separators
    // both survive path comparison and both fail in the shell. Anything that
    // is not byte-for-byte what we would write needs repairing.
    let stale = match (&bridge, current_bridge) {
        (Some(found), Some(current)) => !found.eq_ignore_ascii_case(&command_string(current)),
        _ => false,
    };

    HookStatus {
        project: scope.project_path(),
        path: display,
        exists: true,
        connected: bridge.is_some(),
        bridge,
        stale,
    }
}

/// Add AgentToast's hooks, preserving whatever else is in the file.
pub fn connect(scope: &Scope, bridge: &Path) -> Result<HookStatus, String> {
    let path = scope
        .settings_path()
        .ok_or_else(|| "Could not work out where the settings file lives".to_string())?;

    if !bridge.is_file() {
        return Err(format!(
            "The bridge is not where it was expected: {}",
            bridge.display()
        ));
    }

    let mut settings = read_settings(&path).unwrap_or_else(|| Value::Object(Map::new()));
    if !settings.is_object() {
        return Err(format!(
            "{} does not contain a JSON object; refusing to overwrite it",
            path.display()
        ));
    }

    let command = command_string(bridge);
    let hooks = settings
        .as_object_mut()
        .unwrap()
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()));

    if !hooks.is_object() {
        return Err("The existing \"hooks\" value is not an object".to_string());
    }
    let hooks = hooks.as_object_mut().unwrap();

    apply_hooks(hooks, &command);

    write_settings(&path, &settings)?;
    Ok(status(scope, Some(bridge)))
}

/// Remove AgentToast's hooks and nothing else.
///
/// Worth doing before uninstalling: hooks left pointing at a deleted binary
/// make Claude Code report an error on every single tool call.
pub fn disconnect(scope: &Scope) -> Result<HookStatus, String> {
    let path = scope
        .settings_path()
        .ok_or_else(|| "Could not work out where the settings file lives".to_string())?;

    let Some(mut settings) = read_settings(&path) else {
        return Ok(status(scope, None));
    };

    if let Some(hooks) = settings
        .as_object_mut()
        .and_then(|o| o.get_mut("hooks"))
        .and_then(|h| h.as_object_mut())
    {
        for event in [PERMISSION_REQUEST, SESSION_START, SESSION_END] {
            remove_ours(hooks, event);
        }
        for event in SUPERSEDED_EVENTS {
            remove_ours(hooks, event);
        }
        let empty = hooks.is_empty();
        if empty {
            settings.as_object_mut().unwrap().remove("hooks");
        }
    }

    write_settings(&path, &settings)?;
    Ok(status(scope, None))
}

/* ------------------------------------------------------------- internals --- */

fn read_settings(path: &Path) -> Option<Value> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Write the file, keeping a copy of what was there before.
fn write_settings(path: &Path, settings: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Could not create {}: {}", parent.display(), e))?;
    }

    // Someone else's configuration is about to be rewritten, so leave them a
    // way back. Overwritten each time: the useful copy is the most recent one.
    if path.exists() {
        let backup = path.with_extension("json.agenttoast-backup");
        std::fs::copy(path, &backup)
            .map_err(|e| format!("Could not back up {}: {}", path.display(), e))?;
    }

    let body = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("Could not serialise settings: {}", e))?;
    std::fs::write(path, body + "\n")
        .map_err(|e| format!("Could not write {}: {}", path.display(), e))
}

/// The bridge path referenced anywhere in the settings, if any.
fn find_bridge_command(settings: &Value) -> Option<String> {
    let hooks = settings.get("hooks")?.as_object()?;

    for event in [PERMISSION_REQUEST, SESSION_START, SESSION_END, "PreToolUse"] {
        let Some(groups) = hooks.get(event).and_then(|g| g.as_array()) else {
            continue;
        };
        for group in groups.iter() {
            let Some(entries) = group.get("hooks").and_then(|h| h.as_array()) else {
                continue;
            };
            for entry in entries {
                if let Some(command) = entry.get("command").and_then(|c| c.as_str()) {
                    if command.contains(BRIDGE_MARKER) {
                        return Some(command.to_string());
                    }
                }
            }
        }
    }
    None
}

/// Put AgentToast's hooks into a settings object.
///
/// The whole of what "connected" means, kept in one place so callers — the real
/// one and the tests — cannot drift apart on it.
fn apply_hooks(hooks: &mut Map<String, Value>, command: &str) {
    upsert(hooks, PERMISSION_REQUEST, NO_MATCHER, command);
    upsert(hooks, SESSION_START, Some(SESSION_START_MATCHER), command);
    upsert(hooks, SESSION_END, NO_MATCHER, command);

    // An install predating the switch still has us on PreToolUse. Leaving it
    // would toast twice and keep overriding the user's permission mode.
    for event in SUPERSEDED_EVENTS {
        remove_ours(hooks, event);
    }
}

/// Add our hook to an event, or refresh the path if it is already there.
fn upsert(hooks: &mut Map<String, Value>, event: &str, matcher: Option<&str>, command: &str) {
    let groups = hooks
        .entry(event)
        .or_insert_with(|| Value::Array(Vec::new()));
    if !groups.is_array() {
        *groups = Value::Array(Vec::new());
    }
    let groups = groups.as_array_mut().unwrap();

    // Already present: update the path rather than adding a second copy, so
    // pressing Connect twice is harmless and a reinstall repairs itself.
    for group in groups.iter_mut() {
        let Some(entries) = group.get_mut("hooks").and_then(|h| h.as_array_mut()) else {
            continue;
        };
        for entry in entries.iter_mut() {
            let is_ours = entry
                .get("command")
                .and_then(|c| c.as_str())
                .is_some_and(|c| c.contains(BRIDGE_MARKER));
            if is_ours {
                entry["command"] = json!(command);
                return;
            }
        }
    }

    let mut group = Map::new();
    if let Some(matcher) = matcher {
        group.insert("matcher".into(), json!(matcher));
    }
    group.insert(
        "hooks".into(),
        json!([{ "type": "command", "command": command }]),
    );
    groups.push(Value::Object(group));
}

/// Strip our entries from an event, leaving anyone else's alone.
fn remove_ours(hooks: &mut Map<String, Value>, event: &str) {
    let Some(groups) = hooks.get_mut(event).and_then(|g| g.as_array_mut()) else {
        return;
    };

    for group in groups.iter_mut() {
        if let Some(entries) = group.get_mut("hooks").and_then(|h| h.as_array_mut()) {
            entries.retain(|entry| {
                !entry
                    .get("command")
                    .and_then(|c| c.as_str())
                    .is_some_and(|c| c.contains(BRIDGE_MARKER))
            });
        }
    }

    // A group whose hooks are all gone is noise; so is an event with no groups.
    groups.retain(|group| {
        group
            .get("hooks")
            .and_then(|h| h.as_array())
            .is_none_or(|entries| !entries.is_empty())
    });

    if groups.is_empty() {
        hooks.remove(event);
    }
}

/// Render a path as a command for Claude Code's settings file.
///
/// Two Windows details break the hook if the path is written out verbatim.
///
/// Paths that come back from the OS can carry the `\\?\` extended-length
/// prefix. It is an API detail, not something a shell understands, and it ends
/// up in front of the command as a stray `?`.
///
/// More importantly, Claude Code runs hook commands through bash, where a
/// backslash is an escape character — so `C:\Users\me\tool.exe` arrives as
/// `C:Usersmetool.exe` and cannot be found. Forward slashes are understood by
/// both Windows and the shell, so they are what gets written.
fn command_string(path: &Path) -> String {
    const VERBATIM: &str = r"\\?\";
    const VERBATIM_UNC: &str = r"\\?\UNC\";

    let raw = path.display().to_string();
    let stripped = if let Some(rest) = raw.strip_prefix(VERBATIM_UNC) {
        format!(r"\\{}", rest)
    } else if let Some(rest) = raw.strip_prefix(VERBATIM) {
        rest.to_string()
    } else {
        raw
    };

    stripped.replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hooks_of(settings: &Value) -> &Map<String, Value> {
        settings["hooks"].as_object().unwrap()
    }

    fn connect_into(settings: &mut Value, command: &str) {
        let hooks = settings
            .as_object_mut()
            .unwrap()
            .entry("hooks")
            .or_insert_with(|| Value::Object(Map::new()))
            .as_object_mut()
            .unwrap();
        // Exercise the real thing, so the tests cannot drift from it.
        apply_hooks(hooks, command);
    }

    #[test]
    fn connecting_preserves_unrelated_settings() {
        let mut settings = json!({ "model": "opus", "theme": "dark" });
        connect_into(&mut settings, "C:/app/agenttoast-bridge-claude.exe");

        assert_eq!(settings["model"], "opus");
        assert_eq!(settings["theme"], "dark");
        assert_eq!(hooks_of(&settings).len(), 3);
    }

    #[test]
    fn connecting_leaves_other_peoples_hooks_alone() {
        let mut settings = json!({
            "hooks": {
                "PreToolUse": [
                    { "matcher": "Bash", "hooks": [{ "type": "command", "command": "someone-elses-tool" }] }
                ],
                "PermissionRequest": [
                    { "hooks": [{ "type": "command", "command": "someone-elses-auditor" }] }
                ]
            }
        });
        connect_into(&mut settings, "C:/app/agenttoast-bridge-claude.exe");

        let ours = settings["hooks"][PERMISSION_REQUEST].as_array().unwrap();
        assert_eq!(ours.len(), 2, "ours should be added beside theirs");
        assert_eq!(ours[0]["hooks"][0]["command"], "someone-elses-auditor");

        // Their PreToolUse hook must survive the cleanup of *our* old entries.
        let theirs = settings["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(theirs.len(), 1);
        assert_eq!(theirs[0]["hooks"][0]["command"], "someone-elses-tool");
    }

    /// Upgrading from a version that hooked PreToolUse must move us over, not
    /// leave both wired — otherwise every call toasts twice and the permission
    /// mode is still overridden.
    #[test]
    fn connecting_migrates_off_pretooluse() {
        let mut settings = json!({
            "hooks": {
                "PreToolUse": [
                    { "matcher": "Bash", "hooks": [{ "type": "command", "command": "C:/old/agenttoast-bridge-claude.exe" }] }
                ]
            }
        });
        connect_into(&mut settings, "C:/app/agenttoast-bridge-claude.exe");

        assert!(
            settings["hooks"].get("PreToolUse").is_none(),
            "our old PreToolUse entry should be gone, and the empty event with it"
        );
        assert_eq!(
            settings["hooks"][PERMISSION_REQUEST][0]["hooks"][0]["command"],
            "C:/app/agenttoast-bridge-claude.exe"
        );
    }

    /// Pressing Connect twice must not stack duplicates, and a moved install
    /// must repair the recorded path rather than adding a second entry.
    #[test]
    fn connecting_twice_updates_rather_than_duplicates() {
        let mut settings = json!({});
        connect_into(&mut settings, "C:/old/agenttoast-bridge-claude.exe");
        connect_into(&mut settings, "D:/new/agenttoast-bridge-claude.exe");

        let groups = settings["hooks"][PERMISSION_REQUEST].as_array().unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0]["hooks"][0]["command"],
            "D:/new/agenttoast-bridge-claude.exe"
        );
    }

    #[test]
    fn disconnecting_removes_only_ours() {
        let mut settings = json!({
            "model": "opus",
            "hooks": {
                "PermissionRequest": [
                    { "hooks": [{ "type": "command", "command": "someone-elses-tool" }] }
                ]
            }
        });
        connect_into(&mut settings, "C:/app/agenttoast-bridge-claude.exe");

        let hooks = settings.as_object_mut().unwrap()["hooks"]
            .as_object_mut()
            .unwrap();
        for event in [PERMISSION_REQUEST, SESSION_START, SESSION_END] {
            remove_ours(hooks, event);
        }
        for event in SUPERSEDED_EVENTS {
            remove_ours(hooks, event);
        }

        assert_eq!(settings["model"], "opus");
        let groups = settings["hooks"][PERMISSION_REQUEST].as_array().unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0]["hooks"][0]["command"], "someone-elses-tool");
        assert!(settings["hooks"].get(SESSION_START).is_none());
        assert!(settings["hooks"].get(SESSION_END).is_none());
    }

    #[test]
    fn finds_our_command_whatever_the_path() {
        let mut settings = json!({});
        connect_into(&mut settings, "D:/somewhere/agenttoast-bridge-claude.exe");
        assert_eq!(
            find_bridge_command(&settings).as_deref(),
            Some("D:/somewhere/agenttoast-bridge-claude.exe")
        );
    }

    /// Claude Code runs hooks through bash, which eats backslashes, and the OS
    /// hands back paths carrying a `\\?\` prefix. Written verbatim, the command
    /// arrives as `?C:Usersyou...` and is not found.
    #[test]
    fn command_strips_verbatim_prefix_and_uses_forward_slashes() {
        let path = Path::new(r"\\?\C:\Users\you\tools\agenttoast-bridge-claude.exe");
        assert_eq!(
            command_string(path),
            "C:/Users/you/tools/agenttoast-bridge-claude.exe"
        );
    }

    #[test]
    fn command_keeps_a_plain_path_usable() {
        let path = Path::new(r"D:\apps\AgentToast\agenttoast-bridge-claude.exe");
        assert_eq!(
            command_string(path),
            "D:/apps/AgentToast/agenttoast-bridge-claude.exe"
        );
    }

    #[test]
    fn command_keeps_unc_shares_reachable() {
        let path = Path::new(r"\\?\UNC\server\share\agenttoast-bridge-claude.exe");
        assert_eq!(
            command_string(path),
            "//server/share/agenttoast-bridge-claude.exe"
        );
    }

    /// Naming the right binary is not enough. A command carrying a `\\?\`
    /// prefix or backslash separators points at the correct file and still
    /// fails in the shell, so it has to register as needing repair — otherwise
    /// the dashboard offers "Disconnect" and there is no way to fix it.
    #[test]
    fn a_correct_looking_but_unusable_command_counts_as_stale() {
        let current = Path::new(r"\\?\C:\tools\agenttoast-bridge-claude.exe");
        let written = command_string(current);

        let stored_verbatim = r"\\?\C:\tools\agenttoast-bridge-claude.exe";
        assert!(
            !stored_verbatim.eq_ignore_ascii_case(&written),
            "a verbatim path must be treated as stale"
        );

        assert!(
            "C:/tools/agenttoast-bridge-claude.exe".eq_ignore_ascii_case(&written),
            "an already-correct command must not be flagged"
        );
    }
}

/* --------------------------------------------------------- remembering --- */
//
// Which projects the dashboard should show a row for. Held on disk rather than
// in the window, because a list that vanishes when the window closes is not a
// list — and the dashboard is exactly the thing a user closes and reopens.
//
// Only the *paths* are stored. Whether each is actually connected is always
// re-read from its settings file, so a project edited by hand outside
// AgentToast still shows the truth.

/// Where the remembered project list lives.
fn projects_file() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(".agenttoast").join("projects.json"))
}

/// Projects the user has connected, most recent last.
pub fn remembered_projects() -> Vec<String> {
    let Some(path) = projects_file() else {
        return Vec::new();
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn save_projects(projects: &[String]) {
    let Some(path) = projects_file() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(body) = serde_json::to_string_pretty(projects) {
        let _ = std::fs::write(&path, body + "\n");
    }
}

/// Start showing a row for this project.
pub fn remember_project(project: &str) {
    let mut projects = remembered_projects();
    if !projects.iter().any(|p| p.eq_ignore_ascii_case(project)) {
        projects.push(project.to_string());
        save_projects(&projects);
    }
}

/// Stop showing a row for this project.
pub fn forget_project(project: &str) {
    let mut projects = remembered_projects();
    let before = projects.len();
    projects.retain(|p| !p.eq_ignore_ascii_case(project));
    if projects.len() != before {
        save_projects(&projects);
    }
}
