//! System tray setup.

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    App,
};
use tracing::info;

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

    let _tray = TrayIconBuilder::with_id("agenttoast")
        .icon(icon)
        .menu(&menu)
        .tooltip("AgentToast — Monitoring agent sessions")
        // Left-clicking the tray icon opens the dashboard. Hiding a toast
        // leaves its request pending, so there has to be an obvious way back
        // to it that is quicker than hunting through a menu.
        .on_tray_icon_event(|tray, event| {
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
