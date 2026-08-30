//! AgentToast Remote — answer a toast from your phone.
//!
//! A small HTTP server on the local network, plus the page it serves. A phone
//! on the same wifi opens it, sees whatever is waiting for an answer, and taps
//! Approve or Deny.
//!
//! It is a third client of the two operations the toast and the dashboard
//! already use — read [`SessionRegistry`], call [`ActionRouter::resolve`] — so
//! nothing about the bridges, the hook protocols or the named pipe changes to
//! support it. That is also why it is a separate crate: the network surface
//! should not be able to entangle itself with the local IPC.
//!
//! # Scope
//!
//! **Same network only.** No relay, no account, no server anybody has to run —
//! which also means it does nothing over mobile data. Alerts arrive only while
//! the page is open; a phone in a pocket cannot be woken without an app store
//! and a push service, and that is deliberately out of scope.
//!
//! [`SessionRegistry`]: agenttoast_core::session::SessionRegistry
//! [`ActionRouter::resolve`]: agenttoast_core::router::ActionRouter::resolve

pub mod net;
pub mod pairing;
pub mod qr;
pub mod server;
pub mod store;
pub mod view;

pub use pairing::Pairing;
pub use server::{RemoteState, Running};
pub use store::{DEFAULT_PORT, Device, DeviceInfo, Settings, Store};

use std::net::Ipv4Addr;

/// The URL a pairing QR carries.
///
/// Deliberately an IP address and not a hostname: the server refuses anything
/// addressed by name, because a name is what a DNS-rebinding attack needs. See
/// [`server::host_is_literal`].
pub fn pairing_url(address: Ipv4Addr, port: u16, code: &str) -> String {
    format!("http://{address}:{port}/pair?c={code}")
}

/// The URL a paired device uses from then on.
pub fn home_url(address: Ipv4Addr, port: u16) -> String {
    format!("http://{address}:{port}/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pairing_url_is_addressed_by_ip() {
        let url = pairing_url(Ipv4Addr::new(192, 168, 1, 40), 8787, "deadbeef");
        assert_eq!(url, "http://192.168.1.40:8787/pair?c=deadbeef");
        // The server would reject a hostname, so one must never be built here.
        assert!(server::host_is_literal("192.168.1.40:8787"));
    }

    #[test]
    fn the_home_url_matches_where_pairing_redirects() {
        assert_eq!(
            home_url(Ipv4Addr::new(10, 0, 0, 5), 8787),
            "http://10.0.0.5:8787/"
        );
    }
}
