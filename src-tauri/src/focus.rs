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
//! Two things make this tractable. The ancestor walk stops below the desktop
//! shell: everything the user ever launched is a child of explorer.exe, so
//! expanding it sweeps in the browser and the editor as "related" processes —
//! which is exactly how this used to focus VS Code. And when nothing in the
//! tree owns a real window, hosts are tried in priority order rather than all
//! at once, because searching them together hands the choice to window
//! Z-order, i.e. whichever terminal the user touched last.
//!
//! A console process owns only a titleless ConPTY pseudo-window, which is
//! deliberately skipped — focusing it does nothing visible.
//!
//! Terminal hosts with tabs can only be focused as a whole; there is no
//! supported way to select the specific tab the session is running in.

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
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

/// Applications that host a terminal, used only when the host cannot be
/// identified exactly. Ordered by how likely each is to be the session's home.
const TERMINAL_HOSTS: [&str; 7] = [
    "windowsterminal.exe",
    "conhost.exe",
    "wezterm-gui.exe",
    "alacritty.exe",
    "hyper.exe",
    "kitty.exe",
    "code.exe",
];

/// GUI applications that host a terminal as a descendant process.
const GUI_HOSTS: [&str; 2] = ["code.exe", "windowsterminal.exe"];

/// Bring the window hosting `pid`'s session to the foreground.
///
/// Returns whether a window was actually found and focused.
pub fn focus_agent_window(pid: u32) -> bool {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        // The environment is what identifies a Windows Terminal session.
        ProcessRefreshKind::nothing().with_environ(UpdateKind::Always),
    );

    let agent = Pid::from_u32(pid);
    let ancestors = ancestry(&system, agent);

    // 1. Something in the process tree may own the window outright. That is the
    //    only way to be certain we have the right window.
    let candidates = tree_candidates(&system, &ancestors);
    debug!(agent_pid = pid, ?candidates, "Searching the session's process tree");

    for candidate in candidates {
        if let Some(window) = platform::find_visible_window(candidate) {
            let focused = platform::focus(window);
            info!(
                agent_pid = pid,
                window_pid = candidate,
                focused,
                "Focused the session's terminal window"
            );
            return focused;
        }
    }

    // 2. Otherwise identify the host application and search only its windows.
    let host = identify_host(&system, agent, &ancestors);
    let hosts: Vec<&str> = match host {
        Some(name) => vec![name],
        None => TERMINAL_HOSTS.to_vec(),
    };
    debug!(agent_pid = pid, ?host, ?hosts, "Searching terminal host windows");

    // One host at a time, in priority order. Searching them all at once would
    // hand the decision to window Z-order, i.e. whichever terminal the user
    // touched last — which is how this used to focus the editor instead of the
    // console the agent is actually running in.
    for candidate_host in &hosts {
        let host_pids = pids_named(&system, std::slice::from_ref(candidate_host));
        if host_pids.is_empty() {
            continue;
        }

        let Some((window, host_pid)) = platform::find_topmost_terminal(&host_pids) else {
            continue;
        };

        let focused = platform::focus(window);
        if host.is_some() {
            info!(
                agent_pid = pid,
                window_pid = host_pid,
                host = candidate_host,
                focused,
                "Focused the session's terminal host"
            );
        } else {
            warn!(
                agent_pid = pid,
                window_pid = host_pid,
                host = candidate_host,
                focused,
                "Could not identify the session's host; guessed the most likely terminal"
            );
        }
        return focused;
    }

    warn!(agent_pid = pid, "No terminal window found to focus");
    false
}

/// Which application owns this session's terminal window.
fn identify_host(system: &System, agent: Pid, ancestors: &[Pid]) -> Option<&'static str> {
    // Windows Terminal exports WT_SESSION into processes it launches itself.
    // Note this is absent when Terminal adopts a console as the default
    // terminal application, so its presence is a useful hint but its absence
    // proves nothing.
    if let Some(proc) = system.process(agent) {
        let hosted_by_terminal = proc
            .environ()
            .iter()
            .any(|var| var.to_string_lossy().starts_with("WT_SESSION="));
        if hosted_by_terminal {
            return Some("windowsterminal.exe");
        }
    }

    // Otherwise a GUI host shows up as an ancestor, e.g. the VS Code terminal.
    for pid in ancestors {
        if let Some(name) = process_name(system, *pid) {
            if let Some(host) = GUI_HOSTS.iter().find(|h| **h == name) {
                return Some(host);
            }
        }
    }

    None
}

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

#[cfg(windows)]
mod platform {
    use windows_sys::Win32::Foundation::{FALSE, HWND, LPARAM, TRUE};
    use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, EnumWindows, GetForegroundWindow, GetWindowTextLengthW,
        GetWindowThreadProcessId, IsIconic, IsWindowVisible, SetForegroundWindow, ShowWindow,
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

    /// The frontmost visible window owned by any of `pids`. EnumWindows walks
    /// in Z-order, so the first match is the most recently used one.
    pub fn find_topmost_terminal(pids: &[u32]) -> Option<(HWND, u32)> {
        search(pids.to_vec())
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

    pub fn find_topmost_terminal(_pids: &[u32]) -> Option<((), u32)> {
        None
    }

    pub fn focus(_window: ()) -> bool {
        false
    }
}
