//! Bringing an agent's terminal window to the front.
//!
//! A console agent does not own the window it is displayed in — the host
//! application does (Windows Terminal, the VS Code integrated terminal,
//! conhost), and that host is an ancestor of the agent process. So finding the
//! window to focus means walking up the process tree from the agent's pid and
//! taking the first ancestor that owns a real, visible window.
//!
//! Terminal hosts with tabs can only be focused as a whole; there is no
//! supported way to select the specific tab the session is running in.

use tracing::{info, warn};

/// How far up the process tree to look before giving up.
const MAX_ANCESTRY: usize = 8;

/// Bring the window hosting `pid`'s session to the foreground.
///
/// Returns whether a window was actually found and focused.
pub fn focus_agent_window(pid: u32) -> bool {
    for candidate in ancestry(pid) {
        if let Some(window) = platform::find_visible_window(candidate) {
            let focused = platform::focus(window);
            info!(
                agent_pid = pid,
                window_pid = candidate,
                focused,
                "Focusing agent terminal window"
            );
            return focused;
        }
    }

    warn!(agent_pid = pid, "No window found for the agent or its ancestors");
    false
}

/// The process and its ancestors, nearest first.
fn ancestry(pid: u32) -> Vec<u32> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing(),
    );

    let mut chain = Vec::with_capacity(MAX_ANCESTRY);
    let mut current = Pid::from_u32(pid);

    for _ in 0..MAX_ANCESTRY {
        chain.push(current.as_u32());
        match system.process(current).and_then(|p| p.parent()) {
            Some(parent) if parent != current => current = parent,
            _ => break,
        }
    }

    chain
}

#[cfg(windows)]
mod platform {
    use windows_sys::Win32::Foundation::{FALSE, HWND, LPARAM, TRUE};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextLengthW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
        SetForegroundWindow, ShowWindow, SW_RESTORE,
    };

    struct Search {
        want: u32,
        found: HWND,
    }

    unsafe extern "system" fn enum_proc(window: HWND, lparam: LPARAM) -> i32 {
        let search = unsafe { &mut *(lparam as *mut Search) };

        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(window, &mut pid) };
        if pid != search.want {
            return TRUE;
        }

        // Hosts keep hidden, title-less helper windows around that cannot be
        // focused meaningfully; only a visible titled window is a real target.
        let visible = unsafe { IsWindowVisible(window) } != 0;
        let titled = unsafe { GetWindowTextLengthW(window) } > 0;
        if !visible || !titled {
            return TRUE;
        }

        search.found = window;
        FALSE
    }

    pub fn find_visible_window(pid: u32) -> Option<HWND> {
        let mut search = Search {
            want: pid,
            found: std::ptr::null_mut(),
        };
        unsafe {
            EnumWindows(Some(enum_proc), &mut search as *mut Search as LPARAM);
        }

        if search.found.is_null() {
            None
        } else {
            Some(search.found)
        }
    }

    pub fn focus(window: HWND) -> bool {
        unsafe {
            if IsIconic(window) != 0 {
                ShowWindow(window, SW_RESTORE);
            }
            SetForegroundWindow(window) != 0
        }
    }
}

#[cfg(not(windows))]
mod platform {
    /// Focusing another application's window has no portable equivalent; on
    /// non-Windows platforms "Open Session" falls back to escalating to the
    /// terminal without raising it.
    pub fn find_visible_window(_pid: u32) -> Option<()> {
        None
    }

    pub fn focus(_window: ()) -> bool {
        false
    }
}
