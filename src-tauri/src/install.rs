//! Locating the pieces of an installed AgentToast.
//!
//! There are two programs. The tray app is what the user launches; the bridge
//! is a small binary that Claude Code runs on every tool call, and which talks
//! back to the tray app over the named pipe.
//!
//! Claude Code is told about the bridge by absolute path, so the app has to be
//! able to say where its own bridge lives. That differs between an installed
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

/// Absolute path to the bridge binary, if it can be found.
///
/// Checked in order of trustworthiness: the bundled copy beside the app, then
/// the cargo build output, so a development run works without installing.
pub fn bridge_path(app: &AppHandle) -> Option<PathBuf> {
    let mut tried = Vec::new();

    // Installed: the bundler copies the bridge into the resource directory,
    // which on Windows is the directory holding the executable.
    if let Ok(dir) = app.path().resource_dir() {
        tried.push(dir.join(BRIDGE_EXE));
    }

    // Running from `cargo run`: the app is in target/debug or target/release
    // and the bridge is built alongside it.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            tried.push(dir.join(BRIDGE_EXE));
        }
    }

    tried.into_iter().find(|path| path.is_file())
}
