//! System tray setup.

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    App,
};
use tracing::info;

/// Whether a left click on the tray icon opens the dashboard directly.
#[cfg(not(target_os = "macos"))]
const OPEN_DASHBOARD_ON_LEFT_CLICK: bool = true;
#[cfg(target_os = "macos")]
use crate::mac::runtime::OPEN_DASHBOARD_ON_LEFT_CLICK;

pub fn setup_tray(app: &App) -> Result<(), Box<dyn std::error::Error>> {
    let quit = MenuItem::with_id(app, "quit", "Quit AgentToast", true, None::<&str>)?;
    let show = MenuItem::with_id(app, "show", "Show Dashboard", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    // Without this the tray entry is created with no image at all: it still
    // occupies a slot and still responds to clicks, but renders as blank space,
    // which is impossible to find deliberately.
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or("no application icon is embedded to use in the tray")?;

    let builder = TrayIconBuilder::with_id("agenttoast")
        .icon(icon)
        .menu(&menu)
        .tooltip("AgentToast — Monitoring agent sessions");

    // A macOS menu bar extra opens its menu on a left click, the way Docker and
    // Tailscale do; the dashboard is reached from that menu instead.
    #[cfg(target_os = "macos")]
    let builder = crate::mac::runtime::configure_tray(builder);

    let _tray = builder
        // Left-clicking the tray icon opens the dashboard. Hiding a toast
        // leaves its request pending, so there has to be an obvious way back
        // to it that is quicker than hunting through a menu.
        .on_tray_icon_event(|tray, event| {
            if !OPEN_DASHBOARD_ON_LEFT_CLICK {
                return;
            }
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                crate::window::show_dashboard(tray.app_handle());
            }
        })
        .on_menu_event(|app, event| match event.id.as_ref() {
            "quit" => {
                info!("Quit requested from tray");
                app.exit(0);
            }
            "show" => {
                info!("Show dashboard requested");
                crate::window::show_dashboard(app);
            }
            _ => {}
        })
        .build(app)?;

    info!("System tray initialized");
    Ok(())
}
