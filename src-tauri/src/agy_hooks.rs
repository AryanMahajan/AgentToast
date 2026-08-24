//! Wiring AgentToast into Antigravity's `hooks.json`.
//!
//! The same job [`crate::hooks`] does for Claude Code, and almost none of the
//! same mechanics. Antigravity keeps hooks in their own file rather than inside
//! a general settings file, and the top-level keys there are *hook names* rather
//! than event names — so several tools coexist by each owning one key, and
//! connecting means writing a single `"agenttoast"` object rather than merging
//! entries into arrays somebody else also writes to.
//!
//! Only the global scope is supported. Antigravity's other location for hooks
//! is `<workspace>/.agents/hooks.json`, which is meant to be checked into
//! version control and shared with a team; writing a machine-local absolute
//! path into somebody's repository is not a favour.

use crate::hooks::HookStatus;
use agenttoast_adapters::agy::DEFAULT_MATCHER;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

/// The name AgentToast's hooks are filed under. Other tools use other names,
/// and Antigravity merges them, so this is the whole of what we own.
const HOOK_NAME: &str = "agenttoast";

/// Identifies our own entries whatever path they point at, so a moved or
/// reinstalled AgentToast is recognised and refreshed instead of duplicated.
const BRIDGE_MARKER: &str = "agenttoast-bridge-agy";

/// Seconds Antigravity will wait for the bridge before killing it.
///
/// Must outlast the bridge's own wait for an answer (`bridge_timeout`, 600s by
/// default) so that an unanswered toast ends with the bridge exiting cleanly and
/// writing nothing — which Antigravity reads as "no opinion" — rather than with
/// a killed hook, which fails the tool call outright.
const HOOK_TIMEOUT_SECS: u64 = 660;

/// Where Antigravity's global hooks live.
///
/// This is the "customization root" its documentation refers to: the same
/// directory that holds `mcp_config.json`.
pub fn hooks_path() -> Option<PathBuf> {
    Some(
        dirs::home_dir()?
            .join(".gemini")
            .join("config")
            .join("hooks.json"),
    )
}

/// Read the current state without changing anything.
pub fn status(current_bridge: Option<&Path>) -> HookStatus {
    let Some(path) = hooks_path() else {
        return blank("<unknown>".into(), false);
    };

    let display = path.display().to_string();
    let Some(hooks) = read_hooks(&path) else {
        return blank(display, path.exists());
    };

    let bridge = find_bridge_command(&hooks);

    // An exact comparison against what would be written, not a "do these point
    // at the same file" test: a command can name the right binary and still be
    // unusable, and anything that is not byte-for-byte what we would write now
    // needs repairing.
    let stale = match (&bridge, current_bridge) {
        (Some(found), Some(current)) => match command_string(current) {
            Ok(command) => !found.eq_ignore_ascii_case(&command),
            // The bridge cannot be expressed as a command at all, so whatever
            // is in the file is certainly not it.
            Err(_) => true,
        },
        _ => false,
    };

    HookStatus {
        project: None,
        path: display,
        exists: true,
        connected: bridge.is_some(),
        bridge,
        stale,
    }
}

/// Add AgentToast's hooks, preserving every other named hook in the file.
pub fn connect(bridge: &Path) -> Result<HookStatus, String> {
    let path = hooks_path()
        .ok_or_else(|| "Could not work out where Antigravity keeps its hooks".to_string())?;

    if !bridge.is_file() {
        return Err(format!(
            "The Antigravity bridge is not where it was expected: {}",
            bridge.display()
        ));
    }

    let command = command_string(bridge)?;
    let mut hooks = read_hooks(&path).unwrap_or_default();

    hooks.insert(HOOK_NAME.to_string(), our_hook(&command));

    write_hooks(&path, &hooks)?;
    Ok(status(Some(bridge)))
}

/// Remove AgentToast's hooks and nothing else.
///
/// Worth doing before uninstalling. A hook pointing at a deleted binary does not
/// quietly do nothing in Antigravity — it fails, and a failing `PreToolUse` hook
/// fails the tool call, so every command the agent tries to run is blocked.
pub fn disconnect() -> Result<HookStatus, String> {
    let path = hooks_path()
        .ok_or_else(|| "Could not work out where Antigravity keeps its hooks".to_string())?;

    let Some(mut hooks) = read_hooks(&path) else {
        return Ok(status(None));
    };

    hooks.remove(HOOK_NAME);
    write_hooks(&path, &hooks)?;
    Ok(status(None))
}

/* ------------------------------------------------------------- internals --- */

fn blank(path: String, exists: bool) -> HookStatus {
    HookStatus {
        project: None,
        path,
        exists,
        connected: false,
        bridge: None,
        stale: false,
    }
}

fn read_hooks(path: &Path) -> Option<Map<String, Value>> {
    let raw = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str::<Value>(&raw).ok()? {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

/// Write the file, keeping a copy of what was there before.
fn write_hooks(path: &Path, hooks: &Map<String, Value>) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Could not create {}: {}", parent.display(), e))?;
    }

    if path.exists() {
        let backup = path.with_extension("json.agenttoast-backup");
        std::fs::copy(path, &backup)
            .map_err(|e| format!("Could not back up {}: {}", path.display(), e))?;
    }

    // An empty object is still a valid hooks file, and leaving one behind is
    // tidier than deleting a file Antigravity's own tooling may also write to.
    let body = serde_json::to_string_pretty(&Value::Object(hooks.clone()))
        .map_err(|e| format!("Could not serialise hooks: {}", e))?;
    std::fs::write(path, body + "\n")
        .map_err(|e| format!("Could not write {}: {}", path.display(), e))
}

/// The hook object AgentToast owns.
///
/// The two events are shaped differently, which is easy to get wrong:
/// `PreToolUse` is *grouped*, wrapping its handlers in a `matcher` plus a
/// `hooks` array, while `Stop` is *flat* — handler objects directly, no matcher.
fn our_hook(command: &str) -> Value {
    json!({
        "PreToolUse": [
            {
                "matcher": DEFAULT_MATCHER,
                "hooks": [ handler(command) ]
            }
        ],
        "Stop": [ handler(command) ]
    })
}

fn handler(command: &str) -> Value {
    json!({
        "type": "command",
        "command": command,
        "timeout": HOOK_TIMEOUT_SECS
    })
}

/// The bridge path referenced anywhere in the file, if any.
fn find_bridge_command(hooks: &Map<String, Value>) -> Option<String> {
    fn command_of(handler: &Value) -> Option<&str> {
        handler
            .get("command")
            .and_then(|c| c.as_str())
            .filter(|c| c.contains(BRIDGE_MARKER))
    }

    for named in hooks.values() {
        for event in [
            "PreToolUse",
            "PostToolUse",
            "Stop",
            "PreInvocation",
            "PostInvocation",
        ] {
            let Some(entries) = named.get(event).and_then(|e| e.as_array()) else {
                continue;
            };
            for entry in entries {
                // Flat events hold handlers directly; grouped ones nest them.
                if let Some(command) = command_of(entry) {
                    return Some(command.to_string());
                }
                if let Some(grouped) = entry.get("hooks").and_then(|h| h.as_array()) {
                    if let Some(command) = grouped.iter().find_map(command_of) {
                        return Some(command.to_string());
                    }
                }
            }
        }
    }
    None
}

/// Render a path as a command for Antigravity's hooks file.
///
/// Antigravity splits `command` on whitespace and passes the pieces through
/// without any quote handling — a quoted path arrives at the program with its
/// quotes still attached, and a bare filename is not resolved against the
/// working directory. That leaves no way at all to express a path containing a
/// space, so one is rejected here rather than written out to fail later on
/// every tool call.
///
/// Backslashes are kept. Unlike Claude Code, which runs hooks through bash
/// where a backslash is an escape character, Antigravity runs them through
/// `cmd`, which wants native separators and reads a leading `/` as a switch.
fn command_string(path: &Path) -> Result<String, String> {
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

    if stripped.chars().any(char::is_whitespace) {
        return Err(format!(
            "Antigravity cannot run a hook from a path containing spaces, and \
             AgentToast is installed at {}. Reinstall it somewhere without a \
             space in the path — the default, {}\\AgentToast, is fine — then \
             connect again.",
            stripped,
            dirs::data_local_dir()
                .map(|d| d.display().to_string())
                .unwrap_or_else(|| "%LOCALAPPDATA%".to_string())
        ));
    }

    Ok(stripped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connected_file(command: &str) -> Map<String, Value> {
        let mut hooks = Map::new();
        hooks.insert(HOOK_NAME.to_string(), our_hook(command));
        hooks
    }

    /// `PreToolUse` wraps its handlers in a matcher group; `Stop` does not.
    /// Getting this the wrong way round makes Antigravity ignore the hook.
    #[test]
    fn pre_tool_use_is_grouped_and_stop_is_flat() {
        let hook = our_hook(r"C:\Apps\AgentToast\agenttoast-bridge-agy.exe");

        let group = &hook["PreToolUse"][0];
        assert_eq!(group["matcher"], DEFAULT_MATCHER);
        assert_eq!(group["hooks"][0]["type"], "command");

        let stop = &hook["Stop"][0];
        assert_eq!(stop["type"], "command");
        assert!(
            stop.get("matcher").is_none(),
            "Stop ignores matchers and takes handlers directly"
        );
        assert!(
            stop.get("hooks").is_none(),
            "Stop must not be wrapped in a matcher group"
        );
    }

    /// The bridge waits up to ten minutes for the user. If Antigravity kills it
    /// first the tool call fails outright, instead of falling through to its own
    /// prompt.
    #[test]
    fn the_hook_outlasts_the_bridges_own_wait() {
        let hook = our_hook("bridge.exe");
        let timeout = hook["PreToolUse"][0]["hooks"][0]["timeout"]
            .as_u64()
            .unwrap();

        let bridge_wait = agenttoast_core::config::AppConfig::default()
            .bridge_timeout
            .as_secs();
        assert!(
            timeout > bridge_wait,
            "hook timeout {timeout}s must outlast the bridge's {bridge_wait}s wait"
        );
    }

    #[test]
    fn our_hook_is_found_in_both_shapes() {
        let command = r"C:\Apps\AgentToast\agenttoast-bridge-agy.exe";
        assert_eq!(
            find_bridge_command(&connected_file(command)).as_deref(),
            Some(command)
        );
    }

    /// Other tools file their hooks under their own names, and Antigravity
    /// merges them. Ours must be recognised without matching theirs.
    #[test]
    fn somebody_elses_hook_is_not_mistaken_for_ours() {
        let mut hooks = Map::new();
        hooks.insert(
            "lint-checker".into(),
            json!({ "PostToolUse": [{ "matcher": "*", "hooks": [{ "command": "./lint.sh" }] }] }),
        );
        assert!(find_bridge_command(&hooks).is_none());

        // ...and adding ours leaves theirs in place.
        hooks.insert(
            HOOK_NAME.into(),
            our_hook("bridge/agenttoast-bridge-agy.exe"),
        );
        assert!(hooks.contains_key("lint-checker"));
        assert!(find_bridge_command(&hooks).is_some());

        hooks.remove(HOOK_NAME);
        assert!(hooks.contains_key("lint-checker"));
        assert!(find_bridge_command(&hooks).is_none());
    }

    /// Antigravity runs hooks through `cmd`, which reads a leading `/` as a
    /// switch — the opposite of Claude Code, where backslashes are eaten by
    /// bash and forward slashes are the only thing that works.
    #[test]
    fn separators_are_left_native() {
        let command = command_string(Path::new(r"C:\Apps\AgentToast\agenttoast-bridge-agy.exe"))
            .expect("a path without spaces is fine");
        assert_eq!(command, r"C:\Apps\AgentToast\agenttoast-bridge-agy.exe");
    }

    #[test]
    fn the_verbatim_prefix_is_stripped() {
        let command = command_string(Path::new(r"\\?\C:\Apps\AgentToast\bridge.exe")).unwrap();
        assert_eq!(command, r"C:\Apps\AgentToast\bridge.exe");
    }

    /// There is no quoting that survives Antigravity's whitespace split, so a
    /// path with a space has to be refused up front with something the user can
    /// act on — not written out to fail on every tool call.
    #[test]
    fn a_path_with_a_space_is_refused_with_advice() {
        let error = command_string(Path::new(r"C:\Program Files\AgentToast\bridge.exe"))
            .expect_err("a space makes the command unusable");

        assert!(
            error.contains("Program Files"),
            "say which path is the problem"
        );
        assert!(error.contains("Reinstall"), "say what to do about it");
    }
}
