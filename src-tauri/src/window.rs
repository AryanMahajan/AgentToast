//! Toast window management.

use agenttoast_core::event::AttentionEvent;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tracing::{error, info};

/// Show a toast notification window for an attention event.
pub fn show_toast(
    app: &AppHandle,
    event: &AttentionEvent,
) -> Result<(), Box<dyn std::error::Error>> {
    let label = format!("toast-{}", event.event_id);

    // Check if a toast window already exists for this event
    if app.get_webview_window(&label).is_some() {
        info!("Toast window already exists for event {}", event.event_id);
        return Ok(());
    }

    // Get screen dimensions for positioning
    let (width, height) = (420.0, 220.0);

    let window = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html".into()))
        .title("AgentToast")
        .inner_size(width, height)
        .resizable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .transparent(true)
        .build()?;

    // Position in bottom-right corner
    if let Ok(monitor) = window.primary_monitor() {
        if let Some(monitor) = monitor {
            let screen_size = monitor.size();
            let scale = monitor.scale_factor();
            let x = (screen_size.width as f64 / scale) - width - 20.0;
            let y = (screen_size.height as f64 / scale) - height - 40.0;
            let _ = window.set_position(tauri::Position::Logical(tauri::LogicalPosition {
                x,
                y,
            }));
        }
    }

    info!(
        event_id = %event.event_id,
        "Toast window created"
    );

    Ok(())
}

/// Close a toast window.
pub fn close_toast(
    app: &AppHandle,
    event_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let label = format!("toast-{}", event_id);
    if let Some(window) = app.get_webview_window(&label) {
        window.close()?;
    }
    Ok(())
}
