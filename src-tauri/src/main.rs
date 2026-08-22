//! AgentToast — Cross-agent attention & action router.
//!
//! This is the main Tauri application that runs as a system tray daemon.
//! It listens for attention events from agent bridge scripts via IPC
//! and shows toast notifications for the user to respond to.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod daemon;
mod tray;
mod window;

use agenttoast_core::config::AppConfig;
use agenttoast_core::router::ActionRouter;
use agenttoast_core::session::SessionRegistry;
use std::sync::Arc;
use tracing::info;

/// Shared application state accessible from Tauri commands.
pub struct AppState {
    pub config: AppConfig,
    pub sessions: SessionRegistry,
    pub router: ActionRouter,
    pub auth_token: String,
}

fn main() {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("agenttoast=info".parse().unwrap()),
        )
        .init();

    info!("Starting AgentToast v{}", env!("CARGO_PKG_VERSION"));

    let config = AppConfig::default();
    let sessions = SessionRegistry::new();
    let router = ActionRouter::new();

    // Generate auth token
    let auth_token = agenttoast_ipc::auth::generate_token();
    if let Err(e) = agenttoast_ipc::auth::write_token(&config.data_dir, &auth_token) {
        eprintln!("Failed to write auth token: {}", e);
        std::process::exit(1);
    }

    let state = AppState {
        config,
        sessions,
        router,
        auth_token,
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(Arc::new(state))
        .invoke_handler(tauri::generate_handler![
            commands::get_sessions,
            commands::get_pending_events,
            commands::respond_to_event,
            commands::dismiss_event,
            commands::close_window,
        ])
        .setup(|app| {
            // Set up system tray
            tray::setup_tray(app)?;

            // Start IPC daemon
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                daemon::start(app_handle).await;
            });

            info!("AgentToast setup complete");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Failed to run AgentToast");
}
