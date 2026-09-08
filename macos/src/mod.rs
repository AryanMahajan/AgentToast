//! Everything AgentToast does differently on macOS.
//!
//! The rest of the tree is the Windows application, unchanged. This module is
//! the whole of the macOS port: the shared crates reach it through a handful of
//! one-line `#[cfg(target_os = "macos")]` delegations and nothing else, so a
//! Windows build never compiles a line of it and a Windows behaviour is never
//! altered to make room for a macOS one.
//!
//! | Windows | macOS | Lives in |
//! |---|---|---|
//! | Tray icon, taskbar button | Menu bar extra, no Dock icon | [`runtime`] |
//! | `HWND` per process | `.app` bundle per running application | [`focus`] |
//! | `\\.\pipe\agenttoast` | `~/.agenttoast/agenttoast.sock` | [`bootstrap`] |
//!
//! [`runtime`]: self::runtime
//! [`focus`]: self::focus
//! [`bootstrap`]: self::bootstrap

pub mod bootstrap;
pub mod focus;
pub mod menubar;
pub mod runtime;
