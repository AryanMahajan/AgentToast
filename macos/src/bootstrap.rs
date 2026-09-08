//! Pointing the daemon and the bridges at a Unix socket.
//!
//! `AppConfig`'s built-in default for `ipc.pipe_name` is `\\.\pipe\agenttoast`,
//! a Windows named pipe. That default is correct and is left alone; on macOS it
//! simply names nothing, and a bridge that used it would try to open a file
//! called `\\.\pipe\agenttoast` in whatever directory it happened to start in.
//!
//! `AppConfig::load` already overlays `~/.agenttoast/config.toml` on top of the
//! defaults, and the daemon and every bridge read it — that is the whole point
//! of the file. So the macOS port needs no change to the config type: it just
//! makes sure the overlay exists and names a socket.
//!
//! Deliberately conservative. A `pipe_name` that is already written down is the
//! user's, and is never touched — including one they set to a different socket
//! for a second instance, or a path under `/tmp` to dodge the length limit.

use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// The socket the macOS build listens on, beside the auth token.
///
/// A Unix socket path goes into `sun_path`, which is 104 bytes on macOS — much
/// shorter than `PATH_MAX`, and a silent `EINVAL` at bind time if exceeded. The
/// home directory keeps this near 40 bytes for any ordinary account.
pub fn socket_path(data_dir: &Path) -> PathBuf {
    data_dir.join("agenttoast.sock")
}

/// Make sure `~/.agenttoast/config.toml` names a socket the bridges can reach.
///
/// Returns whether anything was written. Failure is logged rather than fatal:
/// the app still starts, the daemon still reports the endpoint it is using, and
/// the dashboard still explains that no bridge can reach it.
pub fn ensure_socket_endpoint(data_dir: &Path) -> bool {
    let config = data_dir.join(agenttoast_core::config::CONFIG_FILE_NAME);

    let existing = match std::fs::read_to_string(&config) {
        Ok(raw) => Some(raw),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            warn!(path = %config.display(), error = %e, "Could not read the config file");
            return false;
        }
    };

    if existing.as_deref().is_some_and(mentions_pipe_name) {
        return false;
    }

    if let Err(e) = std::fs::create_dir_all(data_dir) {
        warn!(path = %data_dir.display(), error = %e, "Could not create the data directory");
        return false;
    }

    let socket = socket_path(data_dir);
    let mut out = existing.unwrap_or_default();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&section_for(&socket));

    match std::fs::write(&config, out) {
        Ok(()) => {
            info!(
                path = %config.display(),
                socket = %socket.display(),
                "Wrote the macOS IPC endpoint"
            );
            true
        }
        Err(e) => {
            warn!(path = %config.display(), error = %e, "Could not write the config file");
            false
        }
    }
}

/// The `[ipc]` block to append, explaining itself to whoever opens the file.
fn section_for(socket: &Path) -> String {
    format!(
        "\n\
         # Written by AgentToast on macOS. The built-in default is a Windows\n\
         # named pipe, which means nothing here, so the daemon and the bridges\n\
         # agree on a Unix socket instead. Change it and both follow.\n\
         [ipc]\n\
         pipe_name = {}\n",
        toml_string(&socket.display().to_string())
    )
}

/// Whether a config file already sets `ipc.pipe_name`.
///
/// Line-oriented on purpose rather than a TOML parse: the question is only ever
/// "has the user written this key down", and a file too malformed to parse is
/// one this must not silently rewrite either.
fn mentions_pipe_name(raw: &str) -> bool {
    raw.lines()
        .map(|line| line.split('#').next().unwrap_or_default().trim())
        .any(|line| {
            line.strip_prefix("pipe_name")
                .is_some_and(|rest| rest.trim_start().starts_with('='))
        })
}

/// A TOML basic string. Paths cannot contain a NUL and rarely contain a quote,
/// but a home directory is user-controlled and this is written unattended.
fn toml_string(value: &str) -> String {
    let escaped = value.replace('\\', r"\\").replace('"', r#"\""#);
    format!("\"{}\"", escaped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_socket_sits_beside_the_auth_token() {
        let dir = Path::new("/Users/somebody/.agenttoast");
        let socket = socket_path(dir);
        assert_eq!(socket.parent(), Some(dir));
    }

    #[test]
    fn the_socket_path_fits_in_sun_path() {
        // 104 bytes including the terminator, so 103 usable.
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/Users/somebody"));
        let socket = socket_path(&home.join(".agenttoast"));
        assert!(
            socket.display().to_string().len() < 104,
            "{} would not fit in sun_path",
            socket.display()
        );
    }

    #[test]
    fn a_pipe_name_the_user_wrote_is_left_alone() {
        assert!(mentions_pipe_name("[ipc]\npipe_name = \"/tmp/mine.sock\"\n"));
        assert!(mentions_pipe_name("pipe_name=\"/tmp/mine.sock\""));
        assert!(mentions_pipe_name("  pipe_name  =  \"x\"  "));
    }

    #[test]
    fn a_commented_out_pipe_name_is_not_one() {
        assert!(!mentions_pipe_name("[ipc]\n# pipe_name = \"/tmp/mine.sock\"\n"));
        assert!(!mentions_pipe_name(""));
        assert!(!mentions_pipe_name("[escalation]\nenabled = true\n"));
    }

    #[test]
    fn the_written_section_parses_and_round_trips() {
        let socket = PathBuf::from("/Users/somebody/.agenttoast/agenttoast.sock");
        let written = section_for(&socket);
        assert!(mentions_pipe_name(&written));

        let parsed: toml::Value = toml::from_str(&written).expect("valid TOML");
        assert_eq!(
            parsed["ipc"]["pipe_name"].as_str(),
            Some(socket.display().to_string().as_str())
        );
    }

    #[test]
    fn a_quote_in_the_path_cannot_break_the_file() {
        let odd = PathBuf::from("/Users/some\"body/.agenttoast/agenttoast.sock");
        let parsed: toml::Value = toml::from_str(&section_for(&odd)).expect("valid TOML");
        assert_eq!(
            parsed["ipc"]["pipe_name"].as_str(),
            Some(odd.display().to_string().as_str())
        );
    }
}
