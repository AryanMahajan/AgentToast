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
//! [`step_back`] pays it back, by putting whichever application was in front
//! before the toast appeared back in front afterwards. Nothing about answering
//! a toast is a request to switch application. The Dock icon stays; the
//! interruption does not.

use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSApplication, NSApplicationActivationOptions, NSRunningApplication, NSWorkspace,
};
use std::sync::Mutex;
use tauri::tray::TrayIconBuilder;
use tauri::utils::config::WindowEffectsConfig;
use tauri::utils::{WindowEffect, WindowEffectState};
use tauri::{App, AppHandle, Manager, Runtime};
use tracing::{debug, info, warn};

/// Keep AgentToast in the Dock as well as the menu bar.
///
/// Set explicitly rather than left to Tauri's default, because this is a
/// decision with a cost attached — see the module docs, and [`step_back`],
/// which is the other half of it.
pub fn show_in_dock(app: &mut App) {
    app.set_activation_policy(tauri::ActivationPolicy::Regular);
    info!("Running with a Dock icon and a menu bar item");
}

/// Remember which application had the foreground, before a toast can take it.
///
/// Called as a toast is created. Answering it is a click on an AgentToast
/// window, and clicking a regular application's window activates that
/// application — so by the time the answer is in, the information needed to
/// undo that is already gone. It has to be captured up front.
///
/// AgentToast itself is never recorded. If the user was already looking at the
/// dashboard when the toast arrived, then AgentToast being in front afterwards
/// is where they were, not an interruption.
pub fn remember_foreground(app: &AppHandle) {
    // `NSWorkspace` is main-thread only and a toast is created from the IPC
    // daemon's thread, so the read is hopped rather than skipped. Landing a
    // moment later is harmless: the toast window is built unfocused, so it
    // cannot have changed the answer, and the user cannot have clicked it yet.
    let _ = app.run_on_main_thread(|| {
        let pid = NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .map(|app| app.processIdentifier());

        let ours = std::process::id() as i32;
        *PREVIOUS.lock().unwrap() = pid.filter(|p| *p != ours);
    });
}

/// The application that was in front when the last toast appeared.
static PREVIOUS: Mutex<Option<i32>> = Mutex::new(None);

/// Give the foreground back after a toast is answered or dismissed.
///
/// Nothing about answering a toast is a request to switch application, so the
/// application that was in front before it appeared is put back in front. That
/// is a narrower instrument than `NSApplication.hide`, which was what this used
/// to do: hiding takes away *every* window, so an open dashboard would vanish
/// underneath the user for the crime of approving something while it happened
/// to be on screen — and it did nothing at all in that case, because it had to
/// decline whenever any window was visible.
///
/// Falls back to hiding only when there is nobody to hand back to and nothing
/// of ours left on screen, which is the case where hiding is unambiguous.
pub fn step_back(app: &AppHandle) {
    let previous = *PREVIOUS.lock().unwrap();
    let handle = app.clone();

    // Both AppKit calls below are main-thread only, and a toast is closed from
    // the IPC daemon's thread once its bridge has been answered.
    let _ = app.run_on_main_thread(move || {
        // Someone else is already in front — the user moved on while the toast
        // was up, and pulling them anywhere would be the interruption.
        if !frontmost_is_ours() {
            return;
        }

        if let Some(pid) = previous {
            if let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) {
                #[allow(deprecated)]
                let restored =
                    app.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows);
                if restored {
                    return;
                }
                debug!(pid, "Could not reactivate the previous application");
            }
        }

        // Nobody to go back to. Standing aside is still better than staying in
        // front, but only when there is nothing of ours left to look at.
        if !is_showing_something(&handle) {
            if let Err(e) = handle.hide() {
                warn!(error = %e, "Could not stand down");
            }
        }
    });
}

/// Whether AgentToast is the application currently in front.
fn frontmost_is_ours() -> bool {
    NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .is_some_and(|app| app.processIdentifier() == std::process::id() as i32)
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
