//! AgentToast IPC
//!
//! Named pipe (Windows) / Unix socket communication between
//! bridge scripts and the AgentToast daemon.

pub mod auth;
pub mod client;
pub mod protocol;
pub mod server;

pub use protocol::{IpcMessage, IpcResponse};
