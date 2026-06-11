//! Networking helpers for binding and pairing: how the daemon decides which
//! address to listen on and which IP to advertise in a pairing URL.

use std::net::{IpAddr, Ipv4Addr, UdpSocket};

/// Parse a `KAIJU_HOST` value into a bind IP. An unset or unparseable value
/// falls back to loopback (`127.0.0.1`) — the safe localhost-only default.
pub fn bind_ip(host: Option<&str>) -> IpAddr {
    match host.and_then(|h| h.parse::<IpAddr>().ok()) {
        Some(ip) => ip,
        None => IpAddr::V4(Ipv4Addr::LOCALHOST),
    }
}

/// The host to advertise in a pairing URL. Prefers an explicit, routable bind
/// IP (`KAIJU_HOST=192.168.x.x`); otherwise tries to detect the primary LAN IP.
/// Returns `None` when nothing routable is available (e.g. a sandbox) — callers
/// fall back to the request's own Host header.
pub fn advertised_host(bind_host: Option<&str>) -> Option<String> {
    if let Some(ip) = bind_host.and_then(|h| h.parse::<IpAddr>().ok()) {
        if is_routable(ip) {
            return Some(ip.to_string());
        }
    }
    detect_lan_ip().map(|ip| ip.to_string())
}

/// True for an address usable by another device on the network (not loopback,
/// not the unspecified `0.0.0.0`/`::`).
fn is_routable(ip: IpAddr) -> bool {
    !ip.is_loopback() && !ip.is_unspecified()
}

/// Best-effort primary LAN IPv4. Opens a UDP socket "connected" to a public
/// address — no packets are sent; the OS just picks the egress interface, whose
/// local address is our LAN IP. `None` if the OS can't resolve a route.
fn detect_lan_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let local = socket.local_addr().ok()?.ip();
    is_routable(local).then_some(local)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_host_is_loopback() {
        assert_eq!(bind_ip(None), IpAddr::V4(Ipv4Addr::LOCALHOST));
    }

    #[test]
    fn all_interfaces_parses() {
        assert_eq!(bind_ip(Some("0.0.0.0")), IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    }

    #[test]
    fn explicit_lan_ip_parses() {
        assert_eq!(
            bind_ip(Some("192.168.1.5")),
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 5))
        );
    }

    #[test]
    fn garbage_falls_back_to_loopback() {
        assert_eq!(bind_ip(Some("not-an-ip")), IpAddr::V4(Ipv4Addr::LOCALHOST));
    }

    #[test]
    fn advertised_host_prefers_explicit_routable_ip() {
        // An explicit, routable bind IP is what we advertise verbatim.
        assert_eq!(advertised_host(Some("192.168.1.5")), Some("192.168.1.5".to_string()));
    }

    #[test]
    fn advertised_host_ignores_loopback_and_unspecified() {
        // Loopback/unspecified can't go in a QR a phone will open, so we fall
        // through to detection (which may be None in a sandbox) rather than
        // advertising 127.0.0.1 or 0.0.0.0.
        assert_ne!(advertised_host(Some("127.0.0.1")), Some("127.0.0.1".to_string()));
        assert_ne!(advertised_host(Some("0.0.0.0")), Some("0.0.0.0".to_string()));
    }
}
