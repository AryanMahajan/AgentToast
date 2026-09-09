//! Making AgentToast behave like a menu bar app.
//!
//! On Windows AgentToast is a tray application: an icon in the notification
//! area, a taskbar button suppressed per window, and no main window. The macOS
//! equivalents are all set differently, and one of them changes behaviour the
//! user actually notices.
//!
//! # Dock icon *and* menu bar item
//!
//! The obvious choice is `NSApplicationActivationPolicyAccessory` — no Dock
//! icon, menu bar only, the way Docker and Tailscale ship. It was the choice
//! here for a while, and on a Mac with room in the menu bar it is the right
//! one.
//!
//! It is not survivable on a Mac without that room. macOS does not scroll or
//! overflow menu bar items: anything that does not fit between the notch and
//! the clock is **silently not drawn**, with no indicator that it exists. On a
//! notched laptop running an editor with a long menu, several apps' items
//! simply vanish — and an accessory app whose only handle is a menu bar item
//! becomes unreachable, because there is nothing else to click.
//!
//! So [`show_in_dock`] keeps `Regular`. The menu bar item is still created and
//! still works when there is space for it; the Dock icon is the guarantee that
//! there is always *somewhere* to click.
//!
//! # Paying for the Dock icon
//!
//! `Regular` has a cost, and it is the reason `Accessory` was chosen in the
//! first place: activating any window of a regular application activates the
//! whole application. Clicking Approve is a click on an AgentToast window, so
//! macOS brings AgentToast to the front and leaves it there once the toast has
//! gone — which reads as the app opening itself for no reason.
//!
//! [`step_back`] pays it back. Once nothing of ours is on screen any more,
//! AgentToast hides itself, and macOS hands the foreground to whatever the user
//! was using before. The Dock icon stays; the interruption does not.

use objc2::MainThreadMarker;
use tauri::utils::config::WindowEffectsConfig;
use tauri::utils::{WindowEffect, WindowEffectState};
use objc2_app_kit::NSApplication;
use tauri::{App, AppHandle, Manager, Runtime};
use tauri::tray::TrayIconBuilder;
use tracing::{info, warn};

/// Keep AgentToast in the Dock as well as the menu bar.
///
/// Set explicitly rather than left to Tauri's default, because this is a
/// decision with a cost attached — see the module docs, and [`step_back`],
/// which is the other half of it.
pub fn show_in_dock(app: &mut App) {
    app.set_activation_policy(tauri::ActivationPolicy::Regular);
    info!("Running with a Dock icon and a menu bar item");
}

/// Give the foreground back once nothing of ours is on screen.
///
/// Answering a toast means clicking an AgentToast window, and clicking a
/// regular application's window activates that application. Without this the
/// user is left looking at AgentToast every time they approve something.
///
/// Hiding is the whole mechanism: `NSApplication.hide` deactivates us and macOS
/// promotes whatever was in front before, which is where the user actually
/// wants to be. It only runs when there is nothing left to look at — another
/// toast still waiting, or an open dashboard, means hiding would take away a
/// window the user can still see.
pub fn step_back(app: &AppHandle) {
    if is_showing_something(app) {
        return;
    }

    // `hide` is main-thread only, and toasts are closed from the IPC daemon's
    // thread once a bridge is answered.
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        // Re-checked on the main thread: a toast can arrive between the two.
        if is_showing_something(&handle) {
            return;
        }
        if let Err(e) = handle.hide() {
            warn!(error = %e, "Could not give the foreground back");
        }
    });
}

/// Whether any window the user can actually see belongs to us.
///
/// The prewarm webview is deliberately excluded: it exists to pay WebView's
/// startup cost and is parked offscreen, so it must never count as something
/// worth keeping the application in front for.
fn is_showing_something(app: &AppHandle) -> bool {
    app.webview_windows()
        .iter()
        .filter(|(label, _)| label.as_str() != "prewarm")
        .any(|(_, window)| window.is_visible().unwrap_or(false))
}

/// Make the menu bar item behave the way every other one does.
///
/// A menu bar extra opens its menu on a left click — that is the whole
/// convention, and it is what Docker and Tailscale do. On Windows the same
/// icon opens the dashboard on left click and its menu on right, which is that
/// platform's convention; both are reached from the menu here instead.
pub fn configure_tray<R: Runtime>(builder: TrayIconBuilder<R>) -> TrayIconBuilder<R> {
    builder
        .show_menu_on_left_click(true)
        // Replaces the icon the caller set. That one is Tauri's default window
        // icon, which on macOS is the 32×32 PNG built for the Windows tray —
        // see `crate::mac::menubar` for why neither its size nor its kind
        // suits a menu bar.
        .icon(crate::mac::menubar::icon())
        // Which is the other half of it: a template image is drawn from its
        // alpha alone, dark on a light menu bar and light on a dark one. It is
        // what makes Docker's whale and Tailscale's mark legible whatever the
        // user has behind them, and a full-colour icon cannot do it.
        .icon_as_template(true)
}

/// Whether a left click on the menu bar item should also open the dashboard.
///
/// It should not: the click already opens the menu, and doing both means the
/// dashboard appears every time someone glances at the menu.
pub const OPEN_DASHBOARD_ON_LEFT_CLICK: bool = false;

/// Show a window on whichever desktop the user is currently looking at.
///
/// macOS pins a window to the Space it was created on. A toast raised while the
/// user is in another Space would otherwise be invisible until they happened to
/// switch back — for a notification whose whole purpose is to interrupt, that
/// is the same as never showing it. `NSWindowCollectionBehaviorCanJoinAllSpaces`
/// makes it follow them instead.
pub fn follow_the_user<R: Runtime>(window: &tauri::WebviewWindow<R>) {
    if let Err(e) = window.set_visible_on_all_workspaces(true) {
        warn!(error = %e, "Toast will only be visible on one desktop");
    }
}

/// Bring the application forward so a window it is about to show is seen.
///
/// Two separate things have to happen, and neither implies the other.
///
/// `show` is `NSApplication.unhide`, which only undoes a previous `hide`. On an
/// application that was never hidden it does nothing at all — so on its own it
/// leaves the dashboard opening *behind* whatever the user was looking at.
///
/// Activating is the part that raises it. An `Accessory` application has no
/// Dock icon for macOS to activate it through, so it has to ask; that is
/// allowed here because every caller is answering something the user just did
/// — a menu bar click, or clicking the app in Finder.
pub fn present(app: &AppHandle) {
    if let Err(e) = app.show() {
        warn!(error = %e, "Could not unhide AgentToast");
    }

    // `sharedApplication` is main-thread only. Every caller is already on it —
    // a menu event, a tray click, a reopen — but the marker is checked rather
    // than assumed, because being wrong about that is a crash.
    let Some(mtm) = MainThreadMarker::new() else {
        warn!("Not on the main thread; the dashboard may open behind other windows");
        return;
    };

    // Deprecated since macOS 14 in favour of `activate`, which does not exist
    // before it. This one has been there since 10.0 and still works, which
    // matters while the bundle's minimum is 10.15.
    #[allow(deprecated)]
    NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
}

/// Answer a click on the app itself — in Finder, in Launchpad, in Spotlight.
///
/// macOS does not start a second process for an application that is already
/// running: it delivers `applicationShouldHandleReopen:` to the one that is.
/// That means `tauri-plugin-single-instance` never hears about it — it exists
/// to catch a *second process*, and there isn't one — and by default nothing
/// else answers either, so clicking the icon did nothing whatsoever.
///
/// The Dock icon used to paper over this: a `Regular` application at least
/// came to the front when clicked, which read as "it opened". An `Accessory`
/// application has no Dock icon and no such consolation, so the event has to
/// be answered properly.
///
/// `has_visible_windows` is deliberately ignored. A toast on screen makes it
/// true, and "I clicked the app" means the dashboard either way.
pub fn reopen(app: &AppHandle) {
    info!("Reopened from the app icon");
    crate::window::show_dashboard(app);
}

/* ------------------------------------------------------------- appearance --- */

/// Tells the front end it is running on macOS, before the page loads.
///
/// The stylesheet keys its glass treatment off `[data-platform="macos"]`, so
/// Windows renders exactly what it always did — no cascade to fight, no
/// runtime check in a shared component.
///
/// It has to be an injected script rather than an inline `<script>` in the
/// HTML: the app's CSP is `script-src 'self'`, which blocks inline script
/// outright. Injected before page load, so there is no flash of the
/// un-glassed design first.
pub const PLATFORM_SCRIPT: &str = r#"document.documentElement.dataset.platform = "macos";"#;

/// Real macOS vibrancy behind a window.
///
/// `UnderWindowBackground` is the material AppKit uses for a document window's
/// own background — it samples the desktop behind the window rather than
/// tinting a flat colour, which is the whole difference between glass and a
/// translucent rectangle.
///
/// `FollowsWindowActiveState` is what stops it looking wrong when the window is
/// not frontmost: macOS desaturates an inactive window's material, and a pane
/// that stayed vivid while everything around it dimmed would read as a bug.
///
/// The webview above this has to be transparent for any of it to show, which is
/// why the caller pairs it with a transparent window and why the stylesheet
/// leaves `body` unpainted on macOS.
pub fn glass() -> WindowEffectsConfig {
    WindowEffectsConfig {
        effects: vec![WindowEffect::UnderWindowBackground],
        state: Some(WindowEffectState::FollowsWindowActiveState),
        radius: None,
        color: None,
    }
}
