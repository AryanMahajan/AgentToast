//! Taking the user to the terminal an agent is running in, on macOS.
//!
//! This is the whole of "Open Session" on macOS — the search and the raising
//! both. It is a sibling of the Windows implementation in `src-tauri/src/focus.rs`
//! rather than a set of patches to it, because the two platforms disagree about
//! the only question that matters:
//!
//! > *What owns the window the session is displayed in?*
//!
//! On Windows a process owns an `HWND`, and a console agent's terminal may be a
//! *sibling* of the agent (a classic `conhost`) or nowhere in the tree at all
//! (Windows Terminal, started by a broker). On macOS a window belongs to an
//! **application**, an application is a bundle, and the thing in the process
//! tree is some executable buried inside one:
//!
//! ```text
//! Terminal.app:                 Trae / VS Code:
//!   Terminal        <- the app    Electron          <- the app
//!   └── login                     └── Trae Helper   <- a nested helper bundle
//!       └── zsh                       └── zsh
//!           └── claude                    └── claude
//! ```
//!
//! The application is therefore always an **ancestor** of the agent, never a
//! sibling and never a child. That is the difference this file turns on.
//!
//! # Why ancestors only
//!
//! The shared Windows walk also considers each ancestor's *children*, because a
//! `conhost` really can be one. Carried over to macOS that rule raises the
//! wrong application, and reliably so: while a toast is on screen the bridge
//! that raised it is still alive — blocked, waiting for the answer — and it is
//! a child of the agent. Its executable is
//! `/Applications/AgentToast.app/Contents/Resources/agenttoast-bridge-claude`,
//! which resolves to a perfectly good `.app` bundle: AgentToast's own. So the
//! first candidate the walk finds is AgentToast, and "Open Session" brings up
//! AgentToast instead of the terminal.
//!
//! Children are not consulted here at all. [`OURSELVES`] is checked as well, so
//! that no other route back to our own bundle can produce the same result.
//!
//! # Why not `/usr/bin/open`
//!
//! `open` on a running application does not merely activate it: Launch Services
//! delivers a **reopen** Apple Event, and an application is free to answer that
//! by making a new window. Terminals generally do — which is how raising an
//! already-open terminal produced a second, empty one next to the session the
//! user was asking to be taken to.
//!
//! [`NSRunningApplication`] activates without reopening. It also cannot start
//! anything: an application that is not running is not in the list, so a
//! fallback can never launch a terminal the user never opened.
//!
//! The activation itself is allowed because of *when* it happens. macOS refuses
//! to let a background application steal the foreground, but AgentToast is
//! frontmost at this moment — the user just clicked a button on its toast — and
//! a frontmost application may hand activation to another.
//!
//! What is lost is the tab: an application is raised as a whole, and there is
//! no supported way to select the split or tab the session is running in. The
//! Windows side has the same limitation.
//!
//! [`NSRunningApplication`]: objc2_app_kit::NSRunningApplication

use objc2_app_kit::{NSApplicationActivationOptions, NSApplicationActivationPolicy, NSRunningApplication};
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use tracing::{debug, info, warn};

/// How far up the process tree to look before giving up. Matches the Windows
/// walk; a terminal is two or three steps up, an editor four.
const MAX_ANCESTRY: usize = 8;

/// macOS parents every application to `launchd`, so the walk stops there for
/// the same reason the Windows one stops at the desktop shell: one step further
/// and every process on the machine is "related" to every other.
const SHELL_ROOTS: [&str; 2] = ["launchd", "loginwindow"];

/// Applications that own windows but are never what "Open Session" means. The
/// Finder owns the desktop and the Dock owns the Dock, so raising either takes
/// the user somewhere they did not ask to go.
const NEVER_FOCUS: [&str; 3] = ["Finder.app", "Dock.app", "WindowServer.app"];

/// Dedicated terminal applications, most likely first, as the bundle is named
/// on disk. Only consulted when the process tree says nothing, which on macOS
/// means the agent's ancestry was unreadable rather than merely unhelpful.
///
/// Editors that merely contain a terminal are deliberately absent: when an
/// agent runs in one the process tree identifies it exactly, so raising every
/// editor on a guess would only bury the user in windows they did not ask for.
const TERMINAL_BUNDLES: [&str; 8] = [
    "Terminal.app",
    "iTerm.app",
    "Ghostty.app",
    "Warp.app",
    "WezTerm.app",
    "Alacritty.app",
    "kitty.app",
    "Hyper.app",
];

/// `PROC_PIDPATHINFO_MAXSIZE` from `<sys/proc_info.h>`.
const PATH_MAX_SIZE: usize = 4096;

unsafe extern "C" {
    /// Absolute path of a running process's executable.
    ///
    /// From libproc, part of libSystem, so it is always linked and needs no
    /// crate. Returns the number of bytes written, or 0 on failure — the normal
    /// answer for a process owned by another user, and not worth reporting.
    fn proc_pidpath(pid: i32, buffer: *mut c_void, buffersize: u32) -> i32;
}

/// Absolute path of `pid`'s executable, if it can be read.
fn executable_path(pid: u32) -> Option<PathBuf> {
    let mut buffer = vec![0u8; PATH_MAX_SIZE];

    // SAFETY: the buffer is owned here, is `PATH_MAX_SIZE` bytes long, and that
    // same length is what libproc is told it may write.
    let written = unsafe {
        proc_pidpath(
            pid as i32,
            buffer.as_mut_ptr() as *mut c_void,
            PATH_MAX_SIZE as u32,
        )
    };

    if written <= 0 {
        return None;
    }

    buffer.truncate(written as usize);
    String::from_utf8(buffer).ok().map(PathBuf::from)
}

/// The application bundle an executable path belongs to, if any.
///
/// The *outermost* `.app`, not the innermost: an Electron editor runs its
/// terminals from a helper that is itself a bundle, and the helper owns no
/// window. A shell (`/bin/zsh`), the agent (`/opt/homebrew/bin/claude`) and
/// `login` have no `.app` anywhere in their path at all, which is the right
/// answer for them — they own nothing, so the walk keeps going up.
pub fn bundle_of(executable: &Path) -> Option<PathBuf> {
    let mut prefix = PathBuf::new();

    for component in executable.components() {
        prefix.push(component);
        if prefix
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
        {
            return Some(prefix);
        }
    }

    None
}

/// AgentToast's own bundle, resolved once.
///
/// Everything that raises a window checks against this. The bridge is the case
/// that made it necessary — see the module docs — but the dashboard and the
/// toasts are inside the same bundle, and none of them is ever the answer to
/// "take me to my session".
static OURSELVES: OnceLock<Option<PathBuf>> = OnceLock::new();

fn our_bundle() -> Option<&'static Path> {
    OURSELVES
        .get_or_init(|| {
            std::env::current_exe()
                .ok()
                .as_deref()
                .and_then(bundle_of)
        })
        .as_deref()
}

/// Whether a bundle is one this must never raise.
fn is_off_limits(bundle: &Path) -> bool {
    if our_bundle().is_some_and(|ours| ours == bundle) {
        return true;
    }

    bundle
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| NEVER_FOCUS.iter().any(|banned| name.eq_ignore_ascii_case(banned)))
}

/// The agent and its ancestors, nearest first, stopping below `launchd` so the
/// search stays inside this session's own process tree.
fn ancestry(system: &System, agent: Pid) -> Vec<Pid> {
    let mut chain = Vec::with_capacity(MAX_ANCESTRY);
    let mut current = agent;

    for _ in 0..MAX_ANCESTRY {
        let name = system
            .process(current)
            .map(|p| p.name().to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if SHELL_ROOTS.contains(&name.as_str()) {
            break;
        }
        chain.push(current);

        match system.process(current).and_then(|p| p.parent()) {
            Some(parent) if parent != current => current = parent,
            _ => break,
        }
    }

    chain
}

/// The running application for `pid`, if that process is one macOS will show.
///
/// `activationPolicy` is the filter that separates an application from the
/// helper processes it spawns. `Trae Helper` and `Code Helper` are registered
/// running applications in their own right, but they are `.prohibited` — they
/// own no window and cannot be brought forward — so the walk has to keep
/// climbing past them to the `.regular` application that does.
fn presentable(pid: u32) -> Option<objc2::rc::Retained<NSRunningApplication>> {
    // Apple documents NSRunningApplication as thread safe, and none of these
    // touch AppKit's main-thread-only surface, so objc2 exposes them as safe.
    let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid as i32)?;

    if app.activationPolicy() != NSApplicationActivationPolicy::Regular {
        return None;
    }
    Some(app)
}

/// Bring a running application to the front, without reopening it.
///
/// Returns whether AppKit accepted the activation.
fn activate(app: &NSRunningApplication) -> bool {
    // `ActivateAllWindows` is what makes this feel like clicking the Dock icon:
    // the application comes forward with its windows, rather than being made
    // frontmost with everything still buried.
    app.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows)
}

/// The bundle an already-resolved running application lives in.
fn bundle_for(app: &NSRunningApplication) -> Option<PathBuf> {
    executable_path(app.processIdentifier() as u32).and_then(|exe| bundle_of(&exe))
}

/// The application a session is running inside, if the process tree names one.
///
/// Split out from [`focus_agent_window`] so the choice can be inspected without
/// yanking the user's focus to whatever it picked.
///
/// Ancestors only, nearest first. A child is never the answer on macOS, and one
/// particular child is actively the wrong one: while a toast is on screen the
/// bridge that raised it is a live child of the agent, and it lives inside
/// AgentToast's own bundle. See the module docs.
fn application_for(
    system: &System,
    agent: u32,
) -> Option<objc2::rc::Retained<NSRunningApplication>> {
    let ancestors = ancestry(system, Pid::from_u32(agent));
    debug!(agent_pid = agent, ?ancestors, "Walking the session's ancestry");

    for ancestor in &ancestors {
        let candidate = ancestor.as_u32();
        let Some(app) = presentable(candidate) else {
            continue;
        };

        if let Some(bundle) = bundle_for(&app) {
            if is_off_limits(&bundle) {
                debug!(pid = candidate, bundle = %bundle.display(), "Not a session to return to");
                continue;
            }
        }
        return Some(app);
    }

    None
}

/// Bring the application hosting `pid`'s session to the front.
///
/// Returns whether anything was raised.
pub fn focus_agent_window(pid: u32) -> bool {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing(),
    );

    // 1. The application the session is running inside. Always an ancestor on
    //    macOS — see the module docs for why children are not considered.
    if let Some(app) = application_for(&system, pid) {
        let bundle = bundle_for(&app).unwrap_or_default();
        let raised = activate(&app);
        info!(
            agent_pid = pid,
            window_pid = app.processIdentifier(),
            bundle = %bundle.display(),
            raised,
            "Raised the session's application"
        );
        return raised;
    }

    // 2. Nothing in the tree is an application the user can be taken to, which
    //    on macOS means the ancestry was unreadable rather than unhelpful. Raise
    //    every running terminal and let them pick: guessing one would leave the
    //    terminal they actually wanted buried behind whatever they were doing.
    //
    //    Only *running* applications are in the list, so this can never start a
    //    terminal that was not already open.
    let raised = raise_all_terminals(&system);
    if raised > 0 {
        warn!(
            agent_pid = pid,
            raised,
            "Could not identify the session's terminal; raised every open terminal"
        );
        return true;
    }

    warn!(agent_pid = pid, "No application found to raise");
    false
}

/// Activate every running terminal application, best guess last so it lands on
/// top. Returns how many were raised.
fn raise_all_terminals(system: &System) -> usize {
    let mut found: Vec<(usize, i32, objc2::rc::Retained<NSRunningApplication>)> = Vec::new();

    for pid in system.processes().keys() {
        let Some(app) = presentable(pid.as_u32()) else {
            continue;
        };
        let Some(bundle) = bundle_for(&app) else {
            continue;
        };
        let Some(name) = bundle.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(rank) = TERMINAL_BUNDLES
            .iter()
            .position(|wanted| name.eq_ignore_ascii_case(wanted))
        else {
            continue;
        };

        // One application, many processes: a terminal's own helpers can each
        // land here, and activating it once is enough.
        let identity = app.processIdentifier();
        if found.iter().any(|(_, seen, _)| *seen == identity) {
            continue;
        }
        found.push((rank, identity, app));
    }

    // Least likely first, so the most likely terminal is activated last and
    // therefore ends up frontmost.
    found.sort_by_key(|(rank, _, _)| std::cmp::Reverse(*rank));
    for (_, _, app) in &found {
        activate(app);
    }

    found.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bundled_executable_resolves_to_its_application() {
        assert_eq!(
            bundle_of(Path::new("/Applications/iTerm.app/Contents/MacOS/iTerm2")),
            Some(PathBuf::from("/Applications/iTerm.app"))
        );
    }

    #[test]
    fn a_nested_helper_resolves_to_the_application_around_it() {
        // The helper owns no window; the editor does.
        let helper = "/Applications/Trae.app/Contents/Frameworks/\
                      Trae Helper.app/Contents/MacOS/Trae Helper";
        assert_eq!(
            bundle_of(Path::new(helper)),
            Some(PathBuf::from("/Applications/Trae.app"))
        );
    }

    #[test]
    fn an_unbundled_executable_belongs_to_no_application() {
        for exe in ["/bin/zsh", "/opt/homebrew/bin/claude", "/usr/bin/login"] {
            assert_eq!(bundle_of(Path::new(exe)), None, "{exe}");
        }
    }

    #[test]
    fn only_a_dot_app_component_counts() {
        assert_eq!(bundle_of(Path::new("/Users/me/apps/notanapp/run")), None);
        assert_eq!(bundle_of(Path::new("/Users/me/App/run")), None);
    }

    /// The bug this file exists to fix: the bridge is a live child of the agent
    /// while a toast is on screen, and it lives inside AgentToast's own bundle.
    #[test]
    fn the_bridge_resolves_to_our_own_bundle_and_is_refused() {
        let bridge = "/Applications/AgentToast.app/Contents/Resources/agenttoast-bridge-claude";
        let bundle = bundle_of(Path::new(bridge)).expect("the bridge is inside a bundle");
        assert_eq!(bundle, PathBuf::from("/Applications/AgentToast.app"));

        // `is_off_limits` compares against wherever this test binary actually
        // runs from, so assert the rule rather than the deployed path.
        assert!(is_off_limits(&PathBuf::from("/System/Library/CoreServices/Finder.app")));
        assert!(is_off_limits(&PathBuf::from("/System/Library/CoreServices/Dock.app")));
        assert!(!is_off_limits(&PathBuf::from("/Applications/iTerm.app")));
    }

    #[test]
    fn our_own_bundle_is_never_a_target() {
        // Whatever this binary is running from, asking to raise it is refused.
        if let Some(ours) = our_bundle() {
            assert!(is_off_limits(ours));
        }
    }

    #[test]
    fn the_terminal_list_names_bundles_not_executables() {
        // `WezTerm.app` runs `wezterm-gui`; matching on the bundle keeps the
        // list readable and matches what `bundle_of` returns.
        for bundle in TERMINAL_BUNDLES {
            assert!(bundle.ends_with(".app"), "{bundle}");
        }
    }

    /// What "Open Session" would actually raise for the process running this
    /// test, printed rather than asserted: the answer depends on where the test
    /// was started from. Run it deliberately —
    ///
    /// ```text
    /// cargo test -p agenttoast --lib -- --ignored --nocapture mac::focus
    /// ```
    ///
    /// — from a terminal or an editor, and the application named should be the
    /// one you are looking at. It must never be AgentToast.
    #[test]
    #[ignore = "reports on this machine's process tree; run it by hand"]
    fn the_application_chosen_for_this_process_is_not_ourselves() {
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing(),
        );

        match application_for(&system, std::process::id()) {
            Some(app) => {
                let bundle = bundle_for(&app).unwrap_or_default();
                println!("would raise: {} (pid {})", bundle.display(), app.processIdentifier());
                assert!(!is_off_limits(&bundle), "picked a bundle it must never pick");
            }
            None => println!("no application in this process tree; would raise every terminal"),
        }
    }

    #[test]
    fn this_process_can_be_traced_back_to_an_executable() {
        // Proves the libproc binding works on the machine running the tests.
        let me = std::process::id();
        assert!(executable_path(me).is_some_and(|p| p.is_absolute()));
    }
}
