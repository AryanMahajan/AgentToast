//! The allow-rules that make Antigravity's Approve button mean something.
//!
//! Antigravity's hook result cannot grant permission. `decision: "allow"` is
//! read, parsed without complaint, and ignored; so is `permissionOverrides`,
//! and so is a grant written into `settings.json` from inside the hook, because
//! permissions are read once at session start. A hook can only tighten.
//!
//! What it *can* do is be the last word once nothing else is asking. Antigravity
//! resolves permissions from three lists in its global settings, in the strict
//! order **deny > ask > allow**, and a hook returning `force_ask` overrides all
//! three. So AgentToast opens the gate ahead of time and stands in it:
//!
//! | Bridge writes | Antigravity does | The user pressed |
//! | :--- | :--- | :--- |
//! | nothing | runs the call, on the strength of [`GRANTS`] | Approve |
//! | `{"decision":"deny"}` | blocks it, and tells the model why | Deny |
//! | `{"decision":"force_ask"}` | prompts in its own terminal | Approve in session — or nothing, because AgentToast could not be reached |
//!
//! The grants are deliberately no wider than the hook's matcher: every tool
//! they cover is a tool that raises a toast, so nothing is auto-approved that
//! AgentToast would not have asked about. `write_file` implicitly carries
//! `read_file` for the same target, which is the one place the two sets do not
//! line up — reads raise no toast, and with these grants in place they no
//! longer prompt outside the workspace either.
//!
//! **This fails open, and there is no fixing that from inside the hook.** A
//! bridge that exits non-zero, prints something unparseable, or is not there at
//! all is treated by Antigravity as a hook with no opinion — not as a failure —
//! and the grant below then lets the call straight through. Every failure the
//! bridge can *see* is answered with `force_ask` instead of silence, which
//! covers a stopped daemon, a timeout and a broken payload. It cannot cover its
//! own absence. That is why writing these grants is something the user turns on,
//! and why [`disable`] runs on disconnect.

use crate::agy::Watch;
use anyhow::{Context, Result};
use serde_json::{Map, Value};
use std::path::PathBuf;

/// Every allow-rule AgentToast might own.
///
/// `command(*)` covers `run_command`; `write_file(*)` covers the file-editing
/// tools. Antigravity's wildcard is a literal `*` and matches every target in
/// that action's namespace.
///
/// Which of these is actually written depends on the watch scope — see
/// [`grants_for`]. This full list is what [`disable`] takes back, so narrowing
/// the scope cannot leave the wider scope's grant behind.
pub const GRANTS: [&str; 2] = ["command(*)", "write_file(*)"];

/// The grants a given watch scope needs.
///
/// Exactly the actions whose tools raise a toast, so nothing is auto-approved
/// that AgentToast would not have asked about. A `CommandsOnly` session gets no
/// `write_file(*)`, which leaves its file edits to Antigravity's own execution
/// mode — the whole point of choosing that scope.
pub fn grants_for(watch: Watch) -> Vec<&'static str> {
    match watch {
        Watch::CommandsAndEdits => GRANTS.to_vec(),
        Watch::CommandsOnly => vec!["command(*)"],
    }
}

/// Whether an approval on *this* tool would be honoured.
///
/// Finer than the watch scope, and the right question for a toast to ask: a
/// commands-only session still grants `command(*)`, so a `run_command` toast can
/// offer Approve while a file-edit toast — were one somehow raised — could not.
/// Both are answered by looking at what is actually in the allow list.
pub fn approves(tool: &str) -> bool {
    grant_for_tool(tool).is_some_and(is_granted)
}

/// Whether one allow-rule is currently in Antigravity's settings.
pub fn is_granted(grant: &str) -> bool {
    let Some(path) = settings_path() else {
        return false;
    };
    let Ok(settings) = read(&path) else {
        return false;
    };
    allow_list(&settings).iter().any(|entry| entry == grant)
}

/// The allow-rule a tool call needs before a silent hook will let it through.
///
/// `None` for anything not in the matcher: no grant means no honourable Approve,
/// which is the safe answer for a tool we have not thought about.
fn grant_for_tool(tool: &str) -> Option<&'static str> {
    match tool {
        "run_command" => Some("command(*)"),
        "write_to_file" | "create_file" | "replace_file_content" | "edit_notebook"
        | "delete_file" => Some("write_file(*)"),
        _ => None,
    }
}

/// Antigravity's startup execution mode, if it has been written down.
///
/// Advisory only. `--mode` and Shift+Tab both change the live mode without
/// touching this file, so it says what a session *starts* as and nothing about
/// what it is now. Good enough to warn with, never good enough to act on.
pub fn agent_mode() -> Option<String> {
    let path = settings_path()?;
    let settings = read(&path).ok()?;
    settings
        .get("agentMode")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// Where the CLI keeps the settings these rules live in.
///
/// Not the same file as `~/.gemini/settings.json`, which belongs to the IDE.
pub fn settings_path() -> Option<PathBuf> {
    Some(
        dirs::home_dir()?
            .join(".gemini")
            .join("antigravity-cli")
            .join("settings.json"),
    )
}

/// Whether Approve can currently do anything, for the scope being watched.
///
/// Every grant the scope needs has to be present: a half-written list would give
/// the user an Approve button that works for commands and quietly falls back to
/// the terminal for file edits.
pub fn enabled_for(watch: Watch) -> bool {
    let Some(path) = settings_path() else {
        return false;
    };
    let Ok(settings) = read(&path) else {
        return false;
    };
    let allow = allow_list(&settings);
    grants_for(watch)
        .iter()
        .all(|grant| allow.iter().any(|entry| entry == grant))
}

/// Add the grants, leaving everything else in the file alone.
///
/// Returns the rules the user already had that will beat ours, since
/// Antigravity resolves deny and ask ahead of allow and a pre-existing
/// `ask(command(*))` would keep prompting no matter what we write.
pub fn enable(watch: Watch) -> Result<Vec<String>> {
    let path = settings_path().context("Could not work out where Antigravity keeps its settings")?;
    let mut settings = read(&path)?;

    let permissions = settings
        .entry("permissions")
        .or_insert_with(|| Value::Object(Map::new()));
    let permissions = permissions
        .as_object_mut()
        .context("\"permissions\" in Antigravity's settings is not an object")?;

    let allow = permissions
        .entry("allow")
        .or_insert_with(|| Value::Array(Vec::new()));
    let allow = allow
        .as_array_mut()
        .context("\"permissions.allow\" in Antigravity's settings is not a list")?;

    // Narrowing the scope has to remove the wider scope's grant, not just skip
    // adding it — otherwise switching to commands-only would leave
    // `write_file(*)` standing with nothing watching the tools it covers.
    let wanted = grants_for(watch);
    allow.retain(|entry| {
        entry
            .as_str()
            .is_none_or(|s| !GRANTS.contains(&s) || wanted.contains(&s))
    });
    for grant in wanted {
        if !allow.iter().any(|entry| entry.as_str() == Some(grant)) {
            allow.push(Value::String(grant.to_string()));
        }
    }

    write(&path, &settings)?;
    Ok(shadowing_rules(&settings))
}

/// Take the grants back out, leaving any the user added themselves.
pub fn disable() -> Result<()> {
    let path = settings_path().context("Could not work out where Antigravity keeps its settings")?;
    if !path.exists() {
        return Ok(());
    }
    let mut settings = read(&path)?;

    let Some(allow) = settings
        .get_mut("permissions")
        .and_then(|p| p.get_mut("allow"))
        .and_then(|a| a.as_array_mut())
    else {
        return Ok(());
    };

    allow.retain(|entry| !entry.as_str().is_some_and(|s| GRANTS.contains(&s)));
    write(&path, &settings)
}

/// Rules of the user's own that would beat [`GRANTS`], if any.
///
/// Worth surfacing before they are written as well as after: Antigravity
/// resolves deny > ask > allow, so someone with `ask(command(*))` already set
/// would turn approval on and see nothing change.
pub fn shadowing() -> Result<Vec<String>> {
    let path = settings_path().context("Could not work out where Antigravity keeps its settings")?;
    Ok(shadowing_rules(&read(&path)?))
}

/// Rules of the user's own that take precedence over [`GRANTS`].
///
/// Only `deny` and `ask` can, and only where they name an action we grant.
fn shadowing_rules(settings: &Map<String, Value>) -> Vec<String> {
    let actions: Vec<&str> = GRANTS
        .iter()
        .filter_map(|grant| grant.split('(').next())
        .collect();

    ["deny", "ask"]
        .iter()
        .flat_map(|list| {
            settings
                .get("permissions")
                .and_then(|p| p.get(*list))
                .and_then(|v| v.as_array())
                .map(|v| v.as_slice())
                .unwrap_or_default()
        })
        .filter_map(|entry| entry.as_str())
        .filter(|entry| {
            actions
                .iter()
                .any(|action| entry.starts_with(&format!("{}(", action)))
        })
        .map(str::to_string)
        .collect()
}

fn allow_list(settings: &Map<String, Value>) -> Vec<String> {
    settings
        .get("permissions")
        .and_then(|p| p.get("allow"))
        .and_then(|v| v.as_array())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|e| e.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Read the settings, treating a missing file as an empty one.
///
/// A file that exists but will not parse is an error rather than an empty
/// default: overwriting settings we cannot read would lose them.
fn read(path: &std::path::Path) -> Result<Map<String, Value>> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("Could not read {}", path.display()))?;
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    let value: Value = serde_json::from_str(&text)
        .with_context(|| format!("{} is not valid JSON", path.display()))?;
    match value {
        Value::Object(map) => Ok(map),
        _ => anyhow::bail!("{} does not hold a JSON object", path.display()),
    }
}

fn write(path: &std::path::Path, settings: &Map<String, Value>) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Could not create {}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(settings).context("Could not serialize settings")?;
    std::fs::write(path, text).with_context(|| format!("Could not write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// `enable` on a settings file that has never heard of permissions.
    fn grant(mut settings: Map<String, Value>) -> Map<String, Value> {
        let permissions = settings
            .entry("permissions")
            .or_insert_with(|| Value::Object(Map::new()));
        let allow = permissions
            .as_object_mut()
            .unwrap()
            .entry("allow")
            .or_insert_with(|| Value::Array(Vec::new()));
        let allow = allow.as_array_mut().unwrap();
        for g in GRANTS {
            if !allow.iter().any(|e| e.as_str() == Some(g)) {
                allow.push(Value::String(g.to_string()));
            }
        }
        settings
    }

    #[test]
    fn grants_cover_every_action_the_matcher_raises() {
        // If the matcher grows a tool whose action is not granted, Approve
        // silently stops working for it — so the two lists are pinned together.
        let matcher = crate::agy::DEFAULT_MATCHER;
        assert!(matcher.contains("run_command"), "command(*) is for this");
        assert!(
            matcher.contains("write_to_file") && matcher.contains("delete_file"),
            "write_file(*) is for these"
        );
        assert_eq!(GRANTS.len(), 2, "one grant per action, no wider");
    }

    #[test]
    fn granting_twice_does_not_duplicate() {
        let once = grant(Map::new());
        let twice = grant(once.clone());
        assert_eq!(once, twice);
        assert_eq!(allow_list(&twice).len(), GRANTS.len());
    }

    #[test]
    fn granting_keeps_the_users_own_rules() {
        let mut settings = Map::new();
        settings.insert("colorScheme".into(), json!("dark"));
        settings.insert(
            "permissions".into(),
            json!({"allow": ["command(git)"], "deny": ["command(rm -rf)"]}),
        );

        let granted = grant(settings);

        assert_eq!(granted.get("colorScheme"), Some(&json!("dark")));
        assert!(allow_list(&granted).contains(&"command(git)".to_string()));
        assert_eq!(
            granted["permissions"]["deny"],
            json!(["command(rm -rf)"]),
            "deny is not ours to touch"
        );
    }

    #[test]
    fn shadowing_rules_are_reported_so_the_user_is_not_left_guessing() {
        let settings: Map<String, Value> = serde_json::from_value(json!({
            "permissions": {
                "allow": [],
                "ask": ["command(*)", "read_url(evil.com)"],
                "deny": ["write_file(.git/)"]
            }
        }))
        .unwrap();

        let shadowing = shadowing_rules(&settings);

        assert!(shadowing.contains(&"command(*)".to_string()));
        assert!(shadowing.contains(&"write_file(.git/)".to_string()));
        assert!(
            !shadowing.contains(&"read_url(evil.com)".to_string()),
            "read_url is not an action we grant, so it shadows nothing"
        );
    }

    #[test]
    fn enabled_needs_every_grant_not_just_one() {
        let partial: Map<String, Value> =
            serde_json::from_value(json!({"permissions": {"allow": ["command(*)"]}})).unwrap();
        let allow = allow_list(&partial);
        assert!(
            !GRANTS.iter().all(|g| allow.iter().any(|e| e == g)),
            "a half-written list must not count as enabled"
        );
    }
}
