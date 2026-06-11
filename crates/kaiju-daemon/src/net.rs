//! Networking helpers for binding and pairing: how the daemon decides which
//! address to listen on and which IP to advertise in a pairing URL.

use std::net::{IpAddr, Ipv4Addr};

/// Parse a `KAIJU_HOST` value into a bind IP. An unset or unparseable value
/// falls back to loopback (`127.0.0.1`) — the safe localhost-only default.
pub fn bind_ip(host: Option<&str>) -> IpAddr {
    match host.and_then(|h| h.parse::<IpAddr>().ok()) {
        Some(ip) => ip,
        None => IpAddr::V4(Ipv4Addr::LOCALHOST),
    }
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
}
