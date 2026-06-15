//! Reserve a unique dev-server port per isolated agent.
//!
//! Agents that run in their own git worktree often start a dev server
//! (`npm run dev`, `next dev`, ...). Run several at once and they all reach for
//! the same default port and collide. We hand each isolated agent a distinct
//! free port via `PORT`/`KAIJU_AGENT_PORT` so their dev servers coexist.
//!
//! [`allocate`] is the pure selection logic (probe injected, fully tested);
//! [`allocate_port`] wires it to a real TCP bind probe and the configured base.

use std::collections::HashSet;
use std::net::TcpListener;

/// First port to hand out. Override with `KAIJU_PORT_BASE`. Chosen high to stay
/// clear of the common dev ports (3000, 5173, 8080) a developer runs by hand.
const DEFAULT_BASE: u16 = 41000;

/// How many candidates to try before giving up, so a saturated range can't loop
/// forever.
const MAX_ATTEMPTS: u16 = 200;

/// Pure: pick the first port at or above `base` that is neither already
/// reserved (`taken`) nor reported busy by `is_free`. Returns `None` if no
/// candidate within `MAX_ATTEMPTS` qualifies, or if the range overflows.
fn allocate(base: u16, taken: &HashSet<u16>, is_free: impl Fn(u16) -> bool) -> Option<u16> {
    (0..MAX_ATTEMPTS)
        .filter_map(|offset| base.checked_add(offset))
        .find(|port| !taken.contains(port) && is_free(*port))
}

/// True if `port` can currently be bound on loopback (i.e. nothing is listening).
fn is_free(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

/// Reserve a free dev-server port not in `taken` (ports held by other live
/// agents). Returns `None` only if the range is exhausted.
pub fn allocate_port(taken: &HashSet<u16>) -> Option<u16> {
    let base = std::env::var("KAIJU_PORT_BASE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_BASE);
    allocate(base, taken, is_free)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_base_when_free_and_untaken() {
        let taken = HashSet::new();
        assert_eq!(allocate(41000, &taken, |_| true), Some(41000));
    }

    #[test]
    fn skips_taken_ports() {
        let taken = HashSet::from([41000, 41001]);
        assert_eq!(allocate(41000, &taken, |_| true), Some(41002));
    }

    #[test]
    fn skips_busy_ports() {
        let taken = HashSet::new();
        // 41000 is busy, 41001 is free.
        assert_eq!(allocate(41000, &taken, |p| p != 41000), Some(41001));
    }

    #[test]
    fn combines_taken_and_busy() {
        let taken = HashSet::from([41001]);
        // 41000 busy, 41001 taken, 41002 free.
        assert_eq!(allocate(41000, &taken, |p| p != 41000), Some(41002));
    }

    #[test]
    fn none_when_nothing_is_free() {
        let taken = HashSet::new();
        assert_eq!(allocate(41000, &taken, |_| false), None);
    }
}
