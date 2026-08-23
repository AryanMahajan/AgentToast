//! Bringing an agent's terminal window to the front.
//!
//! A console agent does not own the window it is displayed in — the host
//! application does. Where that host sits relative to the agent depends
//! entirely on the terminal:
//!
//! ```text
//! VS Code:              Windows Terminal:
//!   Code.exe              WindowsTerminal.exe   <- owns the window
//!   └── powershell        (started by a broker, NOT an ancestor)
//!       └── claude.exe
//!                         explorer.exe
//!                         └── cmd.exe
//!                             ├── claude.exe    <- agent, no window
//!                             └── conhost.exe   <- headless in ConPTY mode
//! ```
//!
//! Under VS Code the host is an ancestor, so walking up the process tree finds
//! it. Under Windows Terminal nothing in the tree owns a window at all, and no
//! supported API maps a console process to its terminal window.
//!
//! The search runs in three steps, from certain to merely helpful:
//!
//! 1. **The process tree.** If something in it owns a real window, that is the
//!    session's window — no inference needed. The walk stops below the desktop
//!    shell, because everything the user ever launched is a child of
//!    explorer.exe and expanding it sweeps in the browser and the editor.
//!
//! 2. **The console title.** The bridge reports what the agent's console is
//!    called, and terminal hosts mirror the title of the tab they are showing
//!    into their own window title. A unique match is the right window.
//!
//! 3. **Raise every terminal.** When neither works, guessing a single window
//!    is worse than useless: the user is looking at something else entirely,
//!    and picking wrong leaves the terminal they wanted still buried behind it.
//!    So every terminal window is raised above whatever they were doing, best
//!    guess last so it lands on top, and they pick. Terminals may cover each
//!    other, but all of them are now in front of the document.
//!
//! A console process owns only a titleless ConPTY pseudo-window, which is
//! deliberately skipped — focusing it does nothing visible.
//!
//! Terminal hosts with tabs can only be focused as a whole; there is no
//! supported way to select the specific tab the session is running in.

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use tracing::{debug, info, warn};

/// How far up the process tree to look before giving up.
const MAX_ANCESTRY: usize = 8;

/// Never focus these, even though they own windows: the desktop shell owns the
/// taskbar and the wallpaper, so "focusing" it just yanks the user to nothing.
const NEVER_FOCUS: [&str; 2] = ["explorer.exe", "dwm.exe"];

/// Walking past the desktop shell is meaningless: everything the user has ever
/// launched is a child of explorer.exe, so expanding it sweeps in the browser,
/// the editor and every tray applet as "related" processes. The ancestor walk
/// stops here.
const SHELL_ROOTS: [&str; 5] = [
    "explorer.exe",
    "services.exe",
    "wininit.exe",
    "winlogon.exe",
    "svchost.exe",
];

/// Dedicated terminal applications. These are what gets raised when the
/// session's own window cannot be identified.
///
/// Editors that merely contain a terminal are deliberately absent: when an
/// agent runs in one, the process tree identifies it exactly, so raising every
/// editor window on a guess would only bury the user in windows they did not
/// ask for. Ordered by how likely each is to be the session's home.
const TERMINAL_HOSTS: [&str; 6] = [
    "windowsterminal.exe",
    "conhost.exe",
    "wezterm-gui.exe",
    "alacritty.exe",
    "hyper.exe",
    "kitty.exe",
];

/// Lowercase executable name of a process.
fn process_name(system: &System, pid: Pid) -> Option<String> {
    system
        .process(pid)
        .map(|p| p.name().to_string_lossy().to_ascii_lowercase())
}

/// The agent and its ancestors, nearest first, stopping below the desktop
/// shell so the search stays inside this session's own process tree.
fn ancestry(system: &System, agent: Pid) -> Vec<Pid> {
    let mut chain = Vec::with_capacity(MAX_ANCESTRY);
    let mut current = agent;

    for _ in 0..MAX_ANCESTRY {
        let name = process_name(system, current).unwrap_or_default();
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

/// Processes in the tree that might own a window: each ancestor and its
/// children, nearest relationship first. Children matter because a classic
/// conhost is a *sibling* of the agent rather than an ancestor.
fn tree_candidates(system: &System, ancestors: &[Pid]) -> Vec<u32> {
    let mut candidates: Vec<u32> = Vec::new();

    let push = |pid: Pid, out: &mut Vec<u32>| {
        let name = process_name(system, pid).unwrap_or_default();
        if NEVER_FOCUS.contains(&name.as_str()) {
            return;
        }
        let raw = pid.as_u32();
        if !out.contains(&raw) {
            out.push(raw);
        }
    };

    for ancestor in ancestors {
        push(*ancestor, &mut candidates);
        for (child, _) in system
            .processes()
            .iter()
            .filter(|(_, proc)| proc.parent() == Some(*ancestor))
        {
            push(*child, &mut candidates);
        }
    }

    candidates
}

/// Every running process whose executable matches one of `names`, in the order
/// the names were given so preferred hosts are searched first.
fn pids_named(system: &System, names: &[&str]) -> Vec<u32> {
    let mut out = Vec::new();
    for wanted in names {
        for (pid, proc) in system.processes() {
            let name = proc.name().to_string_lossy().to_ascii_lowercase();
            if name == *wanted {
                out.push(pid.as_u32());
            }
        }
    }
    out
}

/// Bring the terminal hosting `pid`'s session to the foreground.
///
/// Returns whether anything was raised.
pub fn focus_agent_window(pid: u32) -> bool {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing(),
    );

    let agent = Pid::from_u32(pid);
    let ancestors = ancestry(&system, agent);

    // 1. Something in the process tree may own the window outright — the only
    //    way to be certain we have the right one. This is the editor case.
    let candidates = tree_candidates(&system, &ancestors);
    debug!(agent_pid = pid, ?candidates, "Searching the session's process tree");

    for candidate in candidates {
        if let Some(window) = platform::find_visible_window(candidate) {
            let focused = platform::focus(window);
            info!(
                agent_pid = pid,
                window_pid = candidate,
                focused,
                "Focused the session's own window"
            );
            return focused;
        }
    }

    // Everything below deals in dedicated terminal windows.
    let terminal_pids = pids_named(&system, &TERMINAL_HOSTS);

    // 2. Raise every terminal and let the user pick. Guessing a single one
    //    would leave the terminal they actually wanted buried behind whatever
    //    they were working on, which is worse than showing all the candidates.
    let raised = platform::raise_all(&terminal_pids);
    if raised > 0 {
        warn!(
            agent_pid = pid,
            raised,
            "Could not identify the session's terminal; raised every terminal window"
        );
        return true;
    }

    warn!(agent_pid = pid, "No terminal window found to raise");
    false
}

#[cfg(windows)]
mod platform {
    use windows_sys::Win32::Foundation::{FALSE, HWND, LPARAM, TRUE};
    use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, EnumWindows, GetForegroundWindow, GetWindowTextLengthW,
        GetWindowThreadProcessId, IsIconic, IsWindowVisible, SetForegroundWindow, SetWindowPos,
        ShowWindow, HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        SW_RESTORE,
    };

    /// Hosts keep hidden, title-less helper windows around that cannot be
    /// focused meaningfully; only a visible titled window is a real target.
    unsafe fn is_real_window(window: HWND) -> bool {
        let visible = unsafe { IsWindowVisible(window) } != 0;
        let titled = unsafe { GetWindowTextLengthW(window) } > 0;
        visible && titled
    }

    struct Search {
        want: Vec<u32>,
        found: HWND,
        found_pid: u32,
    }

    unsafe extern "system" fn enum_proc(window: HWND, lparam: LPARAM) -> i32 {
        let search = unsafe { &mut *(lparam as *mut Search) };

        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(window, &mut pid) };
        if !search.want.contains(&pid) || !unsafe { is_real_window(window) } {
            return TRUE;
        }

        search.found = window;
        search.found_pid = pid;
        FALSE
    }

    fn search(pids: Vec<u32>) -> Option<(HWND, u32)> {
        let mut state = Search {
            want: pids,
            found: std::ptr::null_mut(),
            found_pid: 0,
        };
        unsafe {
            EnumWindows(Some(enum_proc), &mut state as *mut Search as LPARAM);
        }

        if state.found.is_null() {
            None
        } else {
            Some((state.found, state.found_pid))
        }
    }

    pub fn find_visible_window(pid: u32) -> Option<HWND> {
        search(vec![pid]).map(|(window, _)| window)
    }

    struct RaiseSearch {
        want: Vec<u32>,
        found: Vec<HWND>,
    }

    unsafe extern "system" fn enum_raise(window: HWND, lparam: LPARAM) -> i32 {
        let search = unsafe { &mut *(lparam as *mut RaiseSearch) };

        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(window, &mut pid) };
        if search.want.contains(&pid) && unsafe { is_real_window(window) } {
            search.found.push(window);
        }
        TRUE
    }

    /// Bring every window owned by `pids` above the rest of the desktop, and
    /// give focus to the most likely one.
    ///
    /// Returns how many windows were raised. EnumWindows yields front-to-back,
    /// so raising in reverse order leaves the frontmost terminal on top — the
    /// one the user was most recently in, and the best available guess.
    pub fn raise_all(pids: &[u32]) -> usize {
        let mut state = RaiseSearch {
            want: pids.to_vec(),
            found: Vec::new(),
        };
        unsafe {
            EnumWindows(Some(enum_raise), &mut state as *mut RaiseSearch as LPARAM);
        }

        for window in state.found.iter().rev() {
            unsafe {
                if IsIconic(*window) != 0 {
                    ShowWindow(*window, SW_RESTORE);
                }

                // BringWindowToTop on another process's window quietly does
                // nothing unless this process owns the foreground, which it
                // does not. Briefly marking the window topmost and then
                // dropping it back moves it above every ordinary window
                // without asking to activate it, which needs no such right.
                let nudge = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
                SetWindowPos(*window, HWND_TOPMOST, 0, 0, 0, 0, nudge);
                SetWindowPos(*window, HWND_NOTOPMOST, 0, 0, 0, 0, nudge);
            }
        }

        // Focus last so the keyboard lands somewhere sensible.
        if let Some(best) = state.found.first() {
            focus(*best);
        }

        state.found.len()
    }

    pub fn focus(window: HWND) -> bool {
        unsafe {
            if IsIconic(window) != 0 {
                ShowWindow(window, SW_RESTORE);
            }

            // Windows only lets the process that already owns the foreground
            // hand it to someone else. A background app calling
            // SetForegroundWindow on its own just flashes the taskbar button.
            // Attaching to the current foreground thread's input queue borrows
            // that right for the duration of the call.
            let foreground = GetForegroundWindow();
            let foreground_thread = if foreground.is_null() {
                0
            } else {
                GetWindowThreadProcessId(foreground, std::ptr::null_mut())
            };
            let this_thread = GetCurrentThreadId();

            let attached = foreground_thread != 0
                && foreground_thread != this_thread
                && AttachThreadInput(this_thread, foreground_thread, TRUE) != 0;

            BringWindowToTop(window);
            let raised = SetForegroundWindow(window) != 0;

            if attached {
                AttachThreadInput(this_thread, foreground_thread, FALSE);
            }

            raised
        }
    }
}

#[cfg(not(windows))]
mod platform {
    /// Focusing another application's window has no portable equivalent; on
    /// non-Windows platforms "Open Session" hands the decision back to the
    /// terminal without raising it.
    pub fn find_visible_window(_pid: u32) -> Option<()> {
        None
    }

    pub fn raise_all(_pids: &[u32]) -> usize {
        0
    }

    pub fn focus(_window: ()) -> bool {
        false
    }
}
