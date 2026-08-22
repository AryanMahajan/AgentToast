//! Toast window management.
//!
//! Geometry follows the "Agent toasts, light and dark" design: a 372px card
//! anchored 16px from the right edge of the work area and 12px above the
//! taskbar, stacked upward with a 10px gap, newest nearest the taskbar.
//!
//! Windows are created hidden, already sitting at their final anchor, with a
//! fully transparent window+webview background. They are only shown once the
//! frontend has painted and reported its measured height. Creating a toast
//! visible — or at the OS default position, which is screen-centre — is what
//! produced the white flash of an unstyled card in the middle of the screen.

use agenttoast_core::event::AttentionEvent;
use std::sync::Mutex;
use std::time::Duration;
use tauri::window::Color;
use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};
use tracing::{info, warn};

/// Card width from the design spec (logical px).
const CARD_W: f64 = 372.0;

/// Transparent gutter around the card. The window has to be bigger than the
/// card itself so the drop shadow and the 34px slide-in translate have
/// somewhere to live instead of being clipped at the window edge.
const PAD_L: f64 = 26.0;
const PAD_R: f64 = 44.0;
const PAD_T: f64 = 30.0;
const PAD_B: f64 = 30.0;

/// Anchor offsets: 16px in from the right edge, 12px above the taskbar.
const EDGE_R: f64 = 16.0;
const EDGE_B: f64 = 12.0;

/// Gap between stacked cards.
const STACK_GAP: f64 = 10.0;

/// Height assumed before the frontend reports what it actually measured.
const PROVISIONAL_CARD_H: f64 = 150.0;

/// If the frontend never calls `toast_ready`, show the window anyway. Sized for
/// WebView2 cold start, which on a first toast can take well over a second.
const READY_FALLBACK: Duration = Duration::from_millis(2500);

const WIN_W: f64 = CARD_W + PAD_L + PAD_R;

/// Label of the single dashboard window.
const DASHBOARD_LABEL: &str = "dashboard";

/// One toast occupying a slot in the bottom-right stack.
struct ToastSlot {
    label: String,
    card_h: f64,
}

/// The ordered bottom-right toast stack. Oldest first; the newest toast sits
/// nearest the taskbar and the stack grows upward.
#[derive(Default)]
pub struct ToastStack(Mutex<Vec<ToastSlot>>);

/// Logical bounds of the primary monitor's work area (taskbar excluded).
fn work_area(app: &AppHandle) -> Option<(f64, f64, f64)> {
    let monitor = app.primary_monitor().ok().flatten()?;
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let right = (area.position.x as f64 + area.size.width as f64) / scale;
    let bottom = (area.position.y as f64 + area.size.height as f64) / scale;
    let top = area.position.y as f64 / scale;
    Some((right, bottom, top))
}

/// Window origin for the slot at `index`, given the current stack.
fn origin_for(app: &AppHandle, slots: &[ToastSlot], index: usize) -> Option<(f64, f64)> {
    let (right, bottom, top) = work_area(app)?;

    // The card's right edge sits PAD_R in from the window's right edge.
    let x = right - EDGE_R - (WIN_W - PAD_R);

    // Everything newer than this slot is stacked below it.
    let below: f64 = slots[index + 1..]
        .iter()
        .map(|s| s.card_h + STACK_GAP)
        .sum();

    let card_bottom = bottom - EDGE_B - below;
    let y = (card_bottom - slots[index].card_h - PAD_T).max(top);

    Some((x, y))
}

/// Re-anchor every toast in the stack. Called whenever a toast is added,
/// resized to its measured height, or removed.
fn relayout(app: &AppHandle, slots: &[ToastSlot]) {
    for (i, slot) in slots.iter().enumerate() {
        let Some(window) = app.get_webview_window(&slot.label) else {
            continue;
        };
        if let Some((x, y)) = origin_for(app, slots, i) {
            let _ = window.set_size(LogicalSize::new(WIN_W, slot.card_h + PAD_T + PAD_B));
            let _ = window.set_position(LogicalPosition::new(x, y));
        }
    }
}

/// Create a toast notification window for an attention event.
///
/// The window is built hidden and already positioned; `toast_ready` reveals it.
pub fn show_toast(
    app: &AppHandle,
    event: &AttentionEvent,
) -> Result<(), Box<dyn std::error::Error>> {
    let label = format!("toast-{}", event.event_id);

    if app.get_webview_window(&label).is_some() {
        info!("Toast window already exists for event {}", event.event_id);
        return Ok(());
    }

    // Reserve the slot first so the window is born at its final anchor.
    let origin = {
        let stack = app.state::<ToastStack>();
        let mut slots = stack.0.lock().unwrap();
        slots.push(ToastSlot {
            label: label.clone(),
            card_h: PROVISIONAL_CARD_H,
        });
        origin_for(app, &slots, slots.len() - 1)
    };

    let mut builder = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html".into()))
        .title("AgentToast")
        .inner_size(WIN_W, PROVISIONAL_CARD_H + PAD_T + PAD_B)
        .resizable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .transparent(true)
        // Tauri paints its own window shadow behind the webview; on a
        // transparent window that shows up as a grey rectangle.
        .shadow(false)
        // Alpha 0 keeps both the window and the webview layer from painting
        // the default opaque white before the first frame of CSS lands.
        .background_color(Color(0, 0, 0, 0))
        .visible(false);

    if let Some((x, y)) = origin {
        builder = builder.position(x, y);
    }

    let window = builder.build()?;

    // Release the slot however the window goes away.
    let close_app = app.clone();
    let close_label = label.clone();
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Destroyed) {
            release_slot(&close_app, &close_label);
        }
    });

    // Safety net: if the frontend never reports in, reveal the toast anyway
    // rather than leaving an invisible window holding a stack slot.
    let fallback_app = app.clone();
    let fallback_label = label.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(READY_FALLBACK).await;
        if let Some(window) = fallback_app.get_webview_window(&fallback_label) {
            if !window.is_visible().unwrap_or(true) {
                warn!(
                    label = %fallback_label,
                    "Toast frontend never signalled ready; showing anyway"
                );
                let _ = window.show();
            }
        }
    });

    info!(event_id = %event.event_id, "Toast window created (hidden)");
    Ok(())
}

/// The frontend has painted and measured itself: size the window to the card,
/// re-anchor the stack, and reveal it.
pub fn mark_ready(app: &AppHandle, event_id: &str, card_height: f64) {
    let label = format!("toast-{}", event_id);

    {
        let stack = app.state::<ToastStack>();
        let mut slots = stack.0.lock().unwrap();
        if let Some(slot) = slots.iter_mut().find(|s| s.label == label) {
            slot.card_h = card_height.clamp(64.0, 480.0);
        }
        relayout(app, &slots);
    }

    if let Some(window) = app.get_webview_window(&label) {
        info!(label = %label, card_height, "Toast ready; revealing");
        let _ = window.show();
    }
}

/// Drop a toast out of the stack and re-anchor whatever is left.
fn release_slot(app: &AppHandle, label: &str) {
    let stack = app.state::<ToastStack>();
    let mut slots = stack.0.lock().unwrap();
    let before = slots.len();
    slots.retain(|s| s.label != label);
    if slots.len() != before {
        relayout(app, &slots);
    }
}

/// Close a toast window.
pub fn close_toast(app: &AppHandle, event_id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let label = format!("toast-{}", event_id);
    if let Some(window) = app.get_webview_window(&label) {
        window.close()?;
    }
    Ok(())
}

/// Warm up WebView2 at startup.
///
/// The first webview a process creates pays the WebView2 environment
/// initialisation cost — measured at ~2.9s on a cold start here, which the
/// first real toast would otherwise spend sitting invisible while the ready
/// fallback beat it to the punch. An offscreen 1x1 webview pays that once, up
/// front, so actual toasts render promptly.
pub fn prewarm<R: tauri::Runtime, M: Manager<R>>(app: &M) {
    match WebviewWindowBuilder::new(app, "prewarm", WebviewUrl::App("index.html".into()))
        .title("AgentToast (prewarm)")
        .inner_size(1.0, 1.0)
        .position(-32000.0, -32000.0)
        .visible(false)
        .skip_taskbar(true)
        .decorations(false)
        .build()
    {
        Ok(_) => info!("WebView2 prewarm window created"),
        Err(e) => warn!(error = %e, "WebView2 prewarm failed"),
    }
}

/// Nudge the user about a toast they have not answered yet.
///
/// The agent is blocked until they respond, so an unanswered toast that has
/// slipped behind another window is exactly the problem AgentToast exists to
/// solve. Re-assert the window and let the frontend pulse (and optionally
/// chime) rather than stacking up duplicate toasts for the same event.
pub fn remind(app: &AppHandle, event_id: &str, sound: bool) {
    let label = format!("toast-{}", event_id);
    let Some(window) = app.get_webview_window(&label) else {
        return;
    };

    let _ = window.show();
    let _ = window.set_always_on_top(true);
    let _ = window.request_user_attention(Some(tauri::UserAttentionType::Informational));

    use tauri::Emitter;
    let _ = window.emit("toast-reminder", sound);
}

/// Open the dashboard, or focus it if it is already open.
pub fn show_dashboard(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(DASHBOARD_LABEL) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        return;
    }

    match WebviewWindowBuilder::new(
        app,
        DASHBOARD_LABEL,
        WebviewUrl::App("dashboard.html".into()),
    )
    .title("AgentToast — Sessions")
    .inner_size(600.0, 540.0)
    .min_inner_size(420.0, 320.0)
    .resizable(true)
    .build()
    {
        Ok(_) => info!("Dashboard opened"),
        Err(e) => warn!(error = %e, "Failed to open dashboard"),
    }
}
