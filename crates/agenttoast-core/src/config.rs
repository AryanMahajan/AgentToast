//! Application configuration.
//!
//! Defaults are built in; `~/.agenttoast/config.toml` overlays them. The daemon
//! and every bridge call [`AppConfig::load`] so they agree on the pipe name —
//! if they disagreed, bridges would silently fail to reach the daemon.

use crate::escalation::EscalationConfig;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::{info, warn};

/// Name of the overlay file inside the data directory.
pub const CONFIG_FILE_NAME: &str = "config.toml";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub data_dir: PathBuf,
    pub ipc: IpcConfig,
    pub escalation: EscalationConfig,
    pub bridge_timeout: Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcConfig {
    pub pipe_name: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        let data_dir = dirs_default();
        Self {
            data_dir,
            ipc: IpcConfig::default(),
            escalation: EscalationConfig::default(),
            bridge_timeout: Duration::from_secs(600),
        }
    }
}

impl Default for IpcConfig {
    fn default() -> Self {
        Self {
            pipe_name: r"\\.\pipe\agenttoast".to_string(),
        }
    }
}

impl AppConfig {
    /// Load the defaults, overlaid with `~/.agenttoast/config.toml` if present.
    ///
    /// A missing file is normal. A malformed one is reported and then ignored:
    /// a typo in a config file should not stop the daemon from starting, and
    /// must never take down a bridge mid-hook.
    pub fn load() -> Self {
        let config = Self::default();
        let path = config.config_path();

        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return config,
            Err(e) => {
                warn!(path = %path.display(), error = %e, "Could not read config file; using defaults");
                return config;
            }
        };

        match toml::from_str::<ConfigFile>(&raw) {
            Ok(file) => {
                info!(path = %path.display(), "Loaded configuration");
                file.overlay(config)
            }
            Err(e) => {
                warn!(path = %path.display(), error = %e, "Invalid config file; using defaults");
                config
            }
        }
    }

    /// Path of the overlay config file.
    pub fn config_path(&self) -> PathBuf {
        config_path_in(&self.data_dir)
    }
}

fn config_path_in(data_dir: &Path) -> PathBuf {
    data_dir.join(CONFIG_FILE_NAME)
}

/// The on-disk shape.
///
/// Durations are plain seconds here. The runtime types use [`Duration`], which
/// serde expects as a `{ secs, nanos }` table — not something anyone wants to
/// hand-write in a config file.
#[derive(Debug, Default, Deserialize)]
struct ConfigFile {
    bridge_timeout: Option<u64>,
    ipc: Option<IpcSection>,
    escalation: Option<EscalationSection>,
}

#[derive(Debug, Deserialize)]
struct IpcSection {
    pipe_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EscalationSection {
    enabled: Option<bool>,
    reminder_intervals: Option<Vec<u64>>,
    max_reminders: Option<u32>,
    sound_on_reminder: Option<bool>,
}

impl ConfigFile {
    fn overlay(self, mut config: AppConfig) -> AppConfig {
        if let Some(secs) = self.bridge_timeout {
            config.bridge_timeout = Duration::from_secs(secs);
        }
        if let Some(ipc) = self.ipc {
            if let Some(pipe_name) = ipc.pipe_name {
                config.ipc.pipe_name = pipe_name;
            }
        }
        if let Some(esc) = self.escalation {
            if let Some(enabled) = esc.enabled {
                config.escalation.enabled = enabled;
            }
            if let Some(intervals) = esc.reminder_intervals {
                config.escalation.reminder_intervals =
                    intervals.into_iter().map(Duration::from_secs).collect();
            }
            if let Some(max) = esc.max_reminders {
                // 0 documents "unlimited", which the runtime models as None.
                config.escalation.max_reminders = if max == 0 { None } else { Some(max) };
            }
            if let Some(sound) = esc.sound_on_reminder {
                config.escalation.sound_on_reminder = sound;
            }
        }
        config
    }
}

fn dirs_default() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".agenttoast")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlay(src: &str) -> AppConfig {
        toml::from_str::<ConfigFile>(src)
            .expect("config should parse")
            .overlay(AppConfig::default())
    }

    #[test]
    fn empty_file_keeps_defaults() {
        let config = overlay("");
        assert_eq!(config.bridge_timeout, Duration::from_secs(600));
        assert_eq!(config.ipc.pipe_name, r"\\.\pipe\agenttoast");
    }

    #[test]
    fn partial_file_overlays_only_what_it_sets() {
        let config = overlay("bridge_timeout = 30\n");
        assert_eq!(config.bridge_timeout, Duration::from_secs(30));
        // Untouched keys keep their defaults.
        assert!(config.escalation.enabled);
    }

    #[test]
    fn intervals_are_seconds_and_zero_max_means_unlimited() {
        let config = overlay(
            "[escalation]\nreminder_intervals = [5, 10]\nmax_reminders = 0\n",
        );
        assert_eq!(
            config.escalation.reminder_intervals,
            vec![Duration::from_secs(5), Duration::from_secs(10)]
        );
        assert_eq!(config.escalation.max_reminders, None);
    }

    #[test]
    fn nonzero_max_reminders_is_kept() {
        let config = overlay("[escalation]\nmax_reminders = 3\n");
        assert_eq!(config.escalation.max_reminders, Some(3));
    }

    /// The shipped template must actually parse, and `bridge_timeout` must land
    /// at the top level rather than being swallowed by a preceding table.
    #[test]
    fn shipped_default_template_parses() {
        let raw = include_str!("../../../config/default.toml");
        let file: ConfigFile = toml::from_str(raw).expect("default.toml should parse");
        assert_eq!(file.bridge_timeout, Some(600));

        let config = file.overlay(AppConfig::default());
        assert_eq!(config.bridge_timeout, Duration::from_secs(600));
        assert_eq!(config.escalation.reminder_intervals.len(), 3);
    }
}
