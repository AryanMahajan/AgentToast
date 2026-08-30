//! What the remote server remembers between runs.
//!
//! This lives in its own file (`~/.agenttoast/remote.json`) rather than in
//! `config.toml`, because the app writes it. `config.toml` is hand-edited and
//! full of comments, and rewriting it from code would erase them the first time
//! someone flicked a switch in the dashboard.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// File name inside the data directory.
pub const FILE_NAME: &str = "remote.json";

/// Port the server listens on unless told otherwise.
///
/// Above 1024 so it needs no elevation, and not a port anything common wants.
pub const DEFAULT_PORT: u16 = 8787;

/// Everything the remote feature persists.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Whether the server should be listening.
    ///
    /// Off until someone turns it on, deliberately: this is the only part of
    /// AgentToast that accepts connections from another machine.
    pub enabled: bool,
    pub port: u16,
    /// Whether a paired device may approve, as opposed to only denying.
    ///
    /// Denying from a phone can never do damage. Approving runs a command on
    /// the machine, so it gets its own switch — but it defaults on, because a
    /// remote that can only stop things is not the feature anyone wanted.
    pub allow_approve: bool,
    pub devices: Vec<Device>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            port: DEFAULT_PORT,
            allow_approve: true,
            devices: Vec::new(),
        }
    }
}

/// A phone (or anything else) that has been through pairing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    /// Short public identifier, safe to show and to pass back for revocation.
    pub id: String,
    /// The secret this device sends on every request. It never leaves this file
    /// and the device's own cookie jar — in particular it is not part of
    /// [`DeviceInfo`], which is what the dashboard is given.
    pub token: String,
    pub name: String,
    pub paired_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}

/// A device as the dashboard sees it: everything except the secret.
#[derive(Debug, Clone, Serialize)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub paired_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}

impl From<&Device> for DeviceInfo {
    fn from(device: &Device) -> Self {
        Self {
            id: device.id.clone(),
            name: device.name.clone(),
            paired_at: device.paired_at,
            last_seen_at: device.last_seen_at,
        }
    }
}

/// Shared, persisted remote settings.
///
/// Cloning shares the same underlying state — the Tauri commands and the HTTP
/// handlers both hold one, and a device revoked from the dashboard has to stop
/// working on the phone immediately, not at the next restart.
#[derive(Debug, Clone)]
pub struct Store {
    path: Arc<PathBuf>,
    data: Arc<RwLock<Settings>>,
}

impl Store {
    /// Load from the data directory, falling back to defaults.
    ///
    /// A missing file is the normal first-run case. A corrupt one is reported
    /// and then ignored rather than fatal: the worst outcome is that paired
    /// devices have to pair again, which is a minor annoyance next to the app
    /// refusing to start.
    pub fn load(data_dir: &Path) -> Self {
        let path = data_dir.join(FILE_NAME);

        let settings = match std::fs::read_to_string(&path) {
            Ok(raw) => match serde_json::from_str::<Settings>(&raw) {
                Ok(settings) => settings,
                Err(e) => {
                    warn!(
                        path = %path.display(),
                        error = %e,
                        "Unreadable remote settings; starting fresh"
                    );
                    Settings::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Settings::default(),
            Err(e) => {
                warn!(path = %path.display(), error = %e, "Could not read remote settings");
                Settings::default()
            }
        };

        Self {
            path: Arc::new(path),
            data: Arc::new(RwLock::new(settings)),
        }
    }

    /// An in-memory store that never touches disk. For tests.
    #[cfg(test)]
    pub fn ephemeral(settings: Settings) -> Self {
        Self {
            path: Arc::new(PathBuf::new()),
            data: Arc::new(RwLock::new(settings)),
        }
    }

    pub async fn settings(&self) -> Settings {
        self.data.read().await.clone()
    }

    pub async fn devices(&self) -> Vec<DeviceInfo> {
        self.data
            .read()
            .await
            .devices
            .iter()
            .map(DeviceInfo::from)
            .collect()
    }

    /// Change the settings and write them back.
    ///
    /// Everything that mutates goes through here so that no caller can forget
    /// to save. The closure returns whatever the caller needs to see.
    async fn update<T>(&self, change: impl FnOnce(&mut Settings) -> T) -> Result<T> {
        let mut guard = self.data.write().await;
        let out = change(&mut guard);
        let snapshot = guard.clone();
        drop(guard);
        self.write(&snapshot)?;
        Ok(out)
    }

    fn write(&self, settings: &Settings) -> Result<()> {
        // An empty path is the test store, which has nothing to write to.
        if self.path.as_os_str().is_empty() {
            return Ok(());
        }

        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("Could not create the data directory: {}", parent.display())
            })?;
        }

        let raw = serde_json::to_string_pretty(settings)?;
        std::fs::write(self.path.as_path(), raw)
            .with_context(|| format!("Could not write {}", self.path.display()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(
                self.path.as_path(),
                std::fs::Permissions::from_mode(0o600),
            );
        }

        Ok(())
    }

    pub async fn set_enabled(&self, enabled: bool) -> Result<()> {
        self.update(|s| s.enabled = enabled).await
    }

    pub async fn set_allow_approve(&self, allow: bool) -> Result<()> {
        self.update(|s| s.allow_approve = allow).await
    }

    pub async fn set_port(&self, port: u16) -> Result<()> {
        self.update(|s| s.port = port).await
    }

    /// Record a newly paired device and hand back its secret.
    pub async fn add_device(&self, name: String) -> Result<Device> {
        let now = Utc::now();
        let device = Device {
            id: new_secret(4),
            token: new_secret(32),
            name,
            paired_at: now,
            last_seen_at: now,
        };

        let stored = device.clone();
        self.update(move |s| s.devices.push(stored)).await?;
        info!(device = %device.name, id = %device.id, "Paired a remote device");
        Ok(device)
    }

    /// Find the device a token belongs to, and note that it was just used.
    ///
    /// The timestamp is what makes "last seen 2 minutes ago" possible in the
    /// dashboard, which is the only way to tell a device you still carry from
    /// one you replaced a year ago.
    pub async fn authenticate(&self, token: &str) -> Option<Device> {
        // Read first: the common case is a poll every couple of seconds from a
        // page that is already open, and taking the write lock for each one
        // would serialise every request behind a disk write.
        let known = {
            let guard = self.data.read().await;
            guard
                .devices
                .iter()
                .find(|d| crate::pairing::secret_eq(&d.token, token))
                .cloned()
        }?;

        // Only persist once the clock has meaningfully moved on, so an open
        // page does not rewrite the file twice a second forever.
        if Utc::now() - known.last_seen_at > chrono::Duration::seconds(60) {
            let id = known.id.clone();
            let _ = self
                .update(move |s| {
                    if let Some(device) = s.devices.iter_mut().find(|d| d.id == id) {
                        device.last_seen_at = Utc::now();
                    }
                })
                .await;
        }

        Some(known)
    }

    /// Un-pair one device. Returns false if it was already gone.
    pub async fn revoke(&self, id: &str) -> Result<bool> {
        let id = id.to_string();
        self.update(move |s| {
            let before = s.devices.len();
            s.devices.retain(|d| d.id != id);
            s.devices.len() != before
        })
        .await
    }

    /// Un-pair everything.
    pub async fn revoke_all(&self) -> Result<usize> {
        self.update(|s| std::mem::take(&mut s.devices).len()).await
    }
}

/// A cryptographically random hex string, `bytes * 2` characters long.
pub fn new_secret(bytes: usize) -> String {
    let mut rng = rand::rng();
    let raw: Vec<u8> = (0..bytes).map(|_| rng.random::<u8>()).collect();
    hex::encode(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_hex_of_the_requested_length() {
        let secret = new_secret(32);
        assert_eq!(secret.len(), 64);
        assert!(secret.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn two_secrets_differ() {
        assert_ne!(new_secret(32), new_secret(32));
    }

    #[test]
    fn remote_is_off_until_someone_turns_it_on() {
        let settings = Settings::default();
        assert!(!settings.enabled);
        assert!(settings.devices.is_empty());
        // Approve, by contrast, is on — the switch exists to turn it off.
        assert!(settings.allow_approve);
    }

    #[tokio::test]
    async fn only_the_matching_token_authenticates() {
        let store = Store::ephemeral(Settings::default());
        let device = store.add_device("Test phone".into()).await.expect("pairs");

        assert!(store.authenticate(&device.token).await.is_some());
        assert!(store.authenticate("not-the-token").await.is_none());
    }

    #[tokio::test]
    async fn a_revoked_device_stops_authenticating() {
        let store = Store::ephemeral(Settings::default());
        let device = store.add_device("Old phone".into()).await.expect("pairs");

        assert!(store.revoke(&device.id).await.expect("revokes"));
        assert!(store.authenticate(&device.token).await.is_none());
        // Revoking twice is not an error, it just changes nothing.
        assert!(!store.revoke(&device.id).await.expect("no-op"));
    }

    #[tokio::test]
    async fn device_info_carries_no_token() {
        let store = Store::ephemeral(Settings::default());
        let device = store.add_device("Phone".into()).await.expect("pairs");

        let shown = serde_json::to_string(&store.devices().await).expect("serialises");
        assert!(shown.contains(&device.id));
        assert!(!shown.contains(&device.token));
    }
}
