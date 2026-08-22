//! AgentToast Core
//!
//! Core types, session management, event routing, and state tracking
//! for the AgentToast cross-agent notification system.

pub mod config;
pub mod escalation;
pub mod event;
pub mod router;
pub mod session;
pub mod state;

pub use event::{Action, ActionType, AttentionEvent};
pub use session::{AgentType, Session, SessionRegistry};
pub use state::SessionState;
