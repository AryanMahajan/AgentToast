//! Lifecycle for the LAN remote: switch it on, switch it off, pair a phone.
//!
//! The server itself lives in `agenttoast-remote`. This is the part that knows
//! about the running app — when to bind, what to tell the dashboard, and how to
//! make sure a socket is never left listening after someone said stop.

use agenttoast_core::router::ActionRouter;
use agenttoast_core::session::SessionRegistry;
use agenttoast_remote::store::DeviceInfo;
use agenttoast_remote::{Pairing, RemoteState, Running, Store};
use anyhow::Result;
use serde::Serialize;
use tokio::sync::Mutex;
use tracing::{info, warn};

/// The remote feature, as the rest of the app sees it.
pub struct Remote {
    state: RemoteState,
    /// The live server. `None` means nothing is listening — dropping a
    /// [`Running`] shuts it down, so this field and reality cannot disagree.
    running: Mutex<Option<Running>>,
    /// Why it is not listening, when the answer is not "because it is off".
    /// A busy port is the likely case and needs to be visible, not just logged.
    failure: Mutex<Option<String>>,
    /// The QR currently on screen, kept so that a dashboard poll does not
    /// re-render it every few seconds.
    pairing_view: Mutex<Option<PairingView>>,
}

/// Everything the Remote panel draws.
#[derive(Debug, Clone, Serialize)]
pub struct RemoteStatus {
    /// The saved setting.
    pub enabled: bool,
    /// Whether a socket is actually bound right now. These differ when the port
    /// is taken, which is exactly when someone needs to be told.
    pub listening: bool,
    pub port: u16,
    pub allow_approve: bool,
    /// The address to hand out, best guess first. Empty when the machine is off
    /// the network entirely.
    pub addresses: Vec<String>,
    pub devices: Vec<DeviceInfo>,
    pub failure: Option<String>,
    pub pairing: Option<PairingView>,
}

/// An outstanding pairing code, ready to be scanned.
#[derive(Debug, Clone, Serialize)]
pub struct PairingView {
    pub url: String,
    /// Inline SVG. Rendered here rather than in the front end so the dashboard
    /// needs no QR library and works with no network of its own.
    pub qr_svg: String,
    pub expires_at: String,
}

impl Remote {
    /// Load the saved settings. Nothing binds until [`Remote::apply`] runs.
    pub fn load(data_dir: &std::path::Path, sessions: SessionRegistry, router: ActionRouter) -> Self {
        Self {
            state: RemoteState {
                sessions,
                router,
                store: Store::load(data_dir),
                pairing: Pairing::new(),
            },
            running: Mutex::new(None),
            failure: Mutex::new(None),
            pairing_view: Mutex::new(None),
        }
    }

    /// Bring the server in line with the saved setting.
    ///
    /// Called at startup and after any change. Idempotent: enabling something
    /// already listening on the right port leaves it alone rather than dropping
    /// connections to rebind an identical socket.
    pub async fn apply(&self) {
        let settings = self.state.store.settings().await;
        let mut running = self.running.lock().await;

        if !settings.enabled {
            if running.take().is_some() {
                // Any code still on screen was only ever valid against a server
                // that is now gone.
                self.state.pairing.cancel().await;
                *self.pairing_view.lock().await = None;
            }
            *self.failure.lock().await = None;
            return;
        }

        if running.as_ref().is_some_and(|r| r.addr.port() == settings.port) {
            return;
        }

        // Drop the old server before binding, or the rebind hits its own socket.
        *running = None;

        match agenttoast_remote::server::start(self.state.clone(), settings.port).await {
            Ok(server) => {
                *running = Some(server);
                *self.failure.lock().await = None;
            }
            Err(e) => {
                // Chained so the cause reaches the panel: anyhow's Display shows
                // only the outermost message, which here is the least useful one.
                let detail = e
                    .chain()
                    .map(|c| c.to_string())
                    .collect::<Vec<_>>()
                    .join(" — ");
                warn!(error = %detail, port = settings.port, "Could not start the remote server");
                *self.failure.lock().await = Some(detail);
            }
        }
    }

    pub async fn status(&self) -> RemoteStatus {
        let settings = self.state.store.settings().await;

        // A code that has been spent or has expired should take its QR off the
        // dashboard, which is how someone sees that their phone got through.
        let outstanding = self.state.pairing.outstanding().await;
        if outstanding.is_none() {
            *self.pairing_view.lock().await = None;
        }

        RemoteStatus {
            enabled: settings.enabled,
            listening: self.running.lock().await.is_some(),
            port: settings.port,
            allow_approve: settings.allow_approve,
            addresses: agenttoast_remote::net::lan_addresses()
                .into_iter()
                .map(|ip| agenttoast_remote::home_url(ip, settings.port))
                .collect(),
            devices: self.state.store.devices().await,
            failure: self.failure.lock().await.clone(),
            pairing: self.pairing_view.lock().await.clone(),
        }
    }

    pub async fn set_enabled(&self, enabled: bool) -> Result<()> {
        self.state.store.set_enabled(enabled).await?;
        self.apply().await;
        Ok(())
    }

    pub async fn set_allow_approve(&self, allow: bool) -> Result<()> {
        self.state.store.set_allow_approve(allow).await?;
        info!(allow, "Changed whether paired devices may approve");
        Ok(())
    }

    pub async fn set_port(&self, port: u16) -> Result<()> {
        self.state.store.set_port(port).await?;
        self.apply().await;
        Ok(())
    }

    /// Issue a pairing code and render its QR.
    ///
    /// Refuses when nothing is listening: a QR pointing at a closed port is a
    /// worse experience than being told why, because the failure only shows up
    /// on the phone, several steps later.
    pub async fn begin_pairing(&self) -> Result<()> {
        if self.running.lock().await.is_none() {
            anyhow::bail!("Turn the remote on before pairing a device.");
        }

        let settings = self.state.store.settings().await;
        let Some(address) = agenttoast_remote::net::lan_addresses().into_iter().next() else {
            anyhow::bail!("This machine has no network address a phone could reach.");
        };

        let issued = self.state.pairing.issue().await;
        let url = agenttoast_remote::pairing_url(address, settings.port, &issued.code);
        let qr_svg = agenttoast_remote::qr::svg_for(&url)?;

        *self.pairing_view.lock().await = Some(PairingView {
            url,
            qr_svg,
            expires_at: issued.expires_at.to_rfc3339(),
        });

        Ok(())
    }

    pub async fn cancel_pairing(&self) {
        self.state.pairing.cancel().await;
        *self.pairing_view.lock().await = None;
    }

    pub async fn revoke(&self, id: &str) -> Result<bool> {
        let removed = self.state.store.revoke(id).await?;
        if removed {
            info!(device_id = %id, "Revoked a paired device");
        }
        Ok(removed)
    }

    pub async fn revoke_all(&self) -> Result<usize> {
        let count = self.state.store.revoke_all().await?;
        info!(count, "Revoked every paired device");
        Ok(count)
    }
}
