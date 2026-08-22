//! Application configuration.

use crate::escalation::EscalationConfig;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

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

fn dirs_default() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".agenttoast")
}
