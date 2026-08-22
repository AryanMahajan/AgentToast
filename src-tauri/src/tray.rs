//! System tray setup.

use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    App,
};
use tracing::info;

pub fn setup_tray(app: &App) -> Result<(), Box<dyn std::error::Error>> {
    let quit = MenuItem::with_id(app, "quit", "Quit AgentToast", true, None::<&str>)?;
    let show = MenuItem::with_id(app, "show", "Show Dashboard", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    let _tray = TrayIconBuilder::new()
        .menu(&menu)
        .tooltip("AgentToast — Monitoring agent sessions")
        .on_menu_event(|app, event| match event.id.as_ref() {
            "quit" => {
                info!("Quit requested from tray");
                app.exit(0);
            }
            "show" => {
                info!("Show dashboard requested");
                // TODO: Open dashboard window
            }
            _ => {}
        })
        .build(app)?;

    info!("System tray initialized");
    Ok(())
}
