//! Locating the pieces of an installed AgentToast.
//!
//! There is the tray app, which the user launches, and one bridge per agent:
//! a small binary the agent runs on every hook, which talks back to the tray
//! app over the named pipe.
//!
//! Each agent is told about its bridge by absolute path, so the app has to be
//! able to say where its own bridges live. That differs between an installed
//! copy (beside the executable, put there by the bundler) and a development
//! build (in the cargo target directory).

use std::path::PathBuf;
use tauri::{AppHandle, Manager};

/// File name of the Claude Code bridge binary.
pub const BRIDGE_EXE: &str = if cfg!(windows) {
    "agenttoast-bridge-claude.exe"
} else {
    "agenttoast-bridge-claude"
};

/// File name of the Antigravity bridge binary.
pub const AGY_BRIDGE_EXE: &str = if cfg!(windows) {
    "agenttoast-bridge-agy.exe"
} else {
    "agenttoast-bridge-agy"
};

/// Absolute path to the Claude Code bridge, if it can be found.
pub fn bridge_path(app: &AppHandle) -> Option<PathBuf> {
    find_beside_app(app, BRIDGE_EXE)
}

/// Absolute path to the Antigravity bridge, if it can be found.
pub fn agy_bridge_path(app: &AppHandle) -> Option<PathBuf> {
    find_beside_app(app, AGY_BRIDGE_EXE)
}

/// Locate one of our binaries.
///
/// Checked in order of trustworthiness: the bundled copy beside the app, then
/// the cargo build output, so a development run works without installing.
fn find_beside_app(app: &AppHandle, exe: &str) -> Option<PathBuf> {
    let mut tried = Vec::new();

    // Installed: the bundler copies the bridge into the resource directory,
    // which on Windows is the directory holding the executable.
    if let Ok(dir) = app.path().resource_dir() {
        tried.push(dir.join(exe));
    }

    // Running from `cargo run`: the app is in target/debug or target/release
    // and the bridge is built alongside it.
    if let Ok(app_exe) = std::env::current_exe() {
        if let Some(dir) = app_exe.parent() {
            tried.push(dir.join(exe));
        }
    }

    tried.into_iter().find(|path| path.is_file())
}
