//! Working out which address to put in the QR code.
//!
//! The server binds `0.0.0.0`, so it answers on every interface — but the phone
//! needs one concrete address to open, and a developer machine usually has
//! several: wifi, ethernet, WSL, Docker, a VPN, Hyper-V. Guessing wrong sends
//! the user to an address their phone cannot reach, so this offers the whole
//! list with the most likely one first.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};

/// Every non-loopback IPv4 address on the machine, most likely first.
///
/// "Most likely" is whichever interface the default route uses — see
/// [`preferred`]. Anything else follows in whatever order the OS reports.
pub fn lan_addresses() -> Vec<Ipv4Addr> {
    let mut found: Vec<Ipv4Addr> = match if_addrs::get_if_addrs() {
        Ok(interfaces) => interfaces
            .into_iter()
            .filter(|i| !i.is_loopback())
            .filter_map(|i| match i.ip() {
                std::net::IpAddr::V4(v4) => Some(v4),
                std::net::IpAddr::V6(_) => None,
            })
            // A link-local 169.254.x.x means DHCP failed; the interface is up
            // but talks to nobody, so it is never the answer.
            .filter(|v4| !v4.is_link_local())
            .collect(),
        Err(e) => {
            tracing::warn!(error = %e, "Could not enumerate network interfaces");
            Vec::new()
        }
    };

    if let Some(best) = preferred() {
        found.retain(|ip| *ip != best);
        found.insert(0, best);
    }

    found
}

/// The address the machine would use to reach the outside world.
///
/// No packet is sent. Connecting a UDP socket only asks the OS to pick a route
/// and bind a local address, which is exactly the question being asked — and it
/// answers it far better than picking the first interface in the list, which on
/// a machine with Docker or WSL installed is reliably the wrong one.
///
/// The target is from TEST-NET-1, reserved for documentation and routed
/// nowhere, so this cannot accidentally touch a real host.
fn preferred() -> Option<Ipv4Addr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    match socket.local_addr().ok()? {
        SocketAddr::V4(addr) if !addr.ip().is_unspecified() => Some(*addr.ip()),
        _ => None,
    }
}
