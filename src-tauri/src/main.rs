//! AgentToast â€” Cross-agent attention & action router.
//!
//! This is the main Tauri application that runs as a system tray daemon.
//! It listens for attention events from agent bridge scripts via IPC
//! and shows toast notifications for the user to respond to.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod daemon;
mod focus;
mod hooks;
mod install;
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

    let config = AppConfig::load();
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
        // Must be registered first. AgentToast owns a single named pipe and a
        // single tray icon, so a second copy is never useful: it cannot claim
        // the pipe and just leaves a dead icon behind. Launching it again is
        // taken as "show me the app" instead.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            info!("Already running; surfacing the dashboard instead of starting again");
            window::show_dashboard(app);
        }))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(Arc::new(state))
        .manage(window::ToastStack::default())
        .invoke_handler(tauri::generate_handler![
            commands::get_sessions,
            commands::get_pending_events,
            commands::get_event,
            commands::respond_to_event,
            commands::dismiss_event,
            commands::toast_ready,
            commands::reopen_toast,
            commands::hide_toast,
            commands::bridge_path,
            commands::hook_status,
            commands::connect_hooks,
            commands::disconnect_hooks,
            commands::close_window,
        ])
        .setup(|app| {
            // Set up system tray
            tray::setup_tray(app)?;

            // Pay WebView2's one-off startup cost now, not on the first toast.
            window::prewarm(app);

            // Start IPC daemon
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                daemon::start(app_handle).await;
            });

            info!("AgentToast setup complete");
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Failed to build AgentToast")
        .run(|_app, event| {
            // AgentToast lives in the tray with no persistent window. Tauri
            // exits once the last window closes, so dismissing the first toast
            // would otherwise take the daemon down with it. `code` is set only
            // when something called `app.exit(..)` â€” i.e. the tray's Quit item,
            // which should still be allowed through.
            if let tauri::RunEvent::ExitRequested { api, code, .. } = &event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
