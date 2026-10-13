//! Optional bearer-token authentication.
//!
//! Loopback peers (the host machine itself, and the in-process test harness)
//! are always trusted. Remote peers must present the shared `KAIJU_TOKEN` or a
//! registered device token. The decision is a pure, tested function; the
//! middleware is a thin wrapper.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::server::AppState;

/// Pure: may a request proceed? Loopback peers are always trusted — that's the
/// host machine itself (and the in-process test harness, which has no socket).
/// A remote peer must present the shared token or a registered device token.
pub fn authorized(
    is_loopback: bool,
    shared: &Option<String>,
    device_tokens: &[String],
    provided: Option<&str>,
) -> bool {
    if is_loopback {
        return true;
    }
    match provided {
        None => false,
        Some(p) => {
            let matches = |candidate: &str| {
                constant_time_eq::constant_time_eq(candidate.as_bytes(), p.as_bytes())
            };
            shared.as_deref().is_some_and(&matches) || device_tokens.iter().any(|t| matches(t))
        }
    }
}

/// True when the peer is loopback. `None` (no connection info — only happens in
/// the in-process test harness) is treated as loopback.
pub fn is_loopback(peer: Option<SocketAddr>) -> bool {
    peer.is_none_or(|addr| addr.ip().is_loopback())
}

/// The host name of a `Host` header value, without its port. Pure.
fn host_name(host: &str) -> &str {
    if let Some(rest) = host.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(rest);
    }
    match host.rsplit_once(':') {
        Some((name, port)) if port.bytes().all(|b| b.is_ascii_digit()) => name,
        _ => host,
    }
}

/// Pure: could this request have been sent by a web page on another site?
///
/// A browser on the host machine is a loopback peer, so loopback trust alone
/// would hand the whole API to any page the user visits. Two signals rule that
/// out:
/// - `Origin`, when present, must be this server (same authority as `Host`) —
///   browsers always send it on cross-site writes and WebSocket handshakes.
/// - `Host` must be an IP literal or `localhost`, which a DNS-rebinding page
///   (whose `Origin` *does* match its own `Host`) can never present.
///
/// Non-browser clients (the CLI, the pet) send no `Origin` and pass.
pub fn is_cross_site(origin: Option<&str>, host: Option<&str>) -> bool {
    let Some(host) = host else {
        // No Host header: only the in-process test harness. A browser always
        // sends one, so an Origin without it cannot be verified.
        return origin.is_some();
    };
    let name = host_name(host);
    let host_is_literal =
        name.eq_ignore_ascii_case("localhost") || name.parse::<std::net::IpAddr>().is_ok();
    if !host_is_literal {
        return true;
    }
    match origin {
        None => false,
        Some(origin) => {
            let authority = origin
                .strip_prefix("http://")
                .or_else(|| origin.strip_prefix("https://"));
            !authority.is_some_and(|a| a.eq_ignore_ascii_case(host))
        }
    }
}

/// True when the request may use loopback trust: the peer is loopback and the
/// request is not a cross-site one relayed through the host's browser.
pub fn trusted_loopback(peer: Option<SocketAddr>, headers: &axum::http::HeaderMap) -> bool {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    is_loopback(peer) && !is_cross_site(header("origin"), header("host"))
}

/// Extract the token from an `Authorization: Bearer <token>` header value.
pub fn bearer(header: Option<&str>) -> Option<&str> {
    let value = header?.strip_prefix("Bearer ")?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Middleware enforcing [`authorized`]. Public paths are always allowed.
/// Loopback peers bypass token checks. Remote peers must present a valid token.
pub async fn require_auth(
    State(state): State<AppState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path();
    // Public: liveness, the dashboard page, vendored assets, the pairing page
    // and claim endpoint (an unpaired device must reach these), and the
    // terminal WS (which authenticates from its query string).
    if path == "/health"
        || path == "/"
        || path == "/pair"
        || path == "/pair/claim"
        || path.starts_with("/assets/")
        || (path.ends_with("/terminal/ws") && path.contains("/agents/"))
    {
        return next.run(req).await;
    }

    let header = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());
    // Compute an owned token string before `req` is moved into `next.run`.
    let provided_owned: Option<String> = bearer(header).map(|s| s.to_string());

    let loopback = trusted_loopback(peer.map(|c| c.0), req.headers());
    let tokens = state.devices.read().expect("devices lock").tokens();

    if authorized(
        loopback,
        &state.auth_token,
        &tokens,
        provided_owned.as_deref(),
    ) {
        // Only bump last_seen when the token matched a *device* (not the shared
        // token or a loopback request with no token). Reuses the tokens we
        // already read above — no second lock acquisition for the common case.
        if let Some(ref p) = provided_owned {
            // Plain `==` is fine here (unlike `authorized`, which is
            // constant-time): this runs only *after* auth succeeded, so the
            // caller already holds a valid token — the comparison leaks nothing.
            if tokens.iter().any(|t| t == p) {
                state
                    .devices
                    .write()
                    .expect("devices lock")
                    .touch_by_token(p, chrono::Utc::now());
            }
        }
        next.run(req).await
    } else {
        (StatusCode::UNAUTHORIZED, "unauthorized").into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_is_always_trusted() {
        assert!(authorized(true, &None, &[], None));
        assert!(authorized(true, &Some("secret".into()), &[], None));
    }

    #[test]
    fn remote_without_token_is_rejected() {
        assert!(!authorized(false, &None, &[], None));
        assert!(!authorized(
            false,
            &Some("secret".into()),
            &[],
            Some("wrong")
        ));
        assert!(!authorized(false, &None, &["dev-a".into()], Some("dev-b")));
    }

    #[test]
    fn remote_with_shared_token_is_allowed() {
        assert!(authorized(
            false,
            &Some("secret".into()),
            &[],
            Some("secret")
        ));
    }

    #[test]
    fn remote_with_device_token_is_allowed() {
        let devices = vec!["dev-a".to_string(), "dev-b".to_string()];
        assert!(authorized(false, &None, &devices, Some("dev-b")));
    }

    #[test]
    fn loopback_helper_treats_none_and_loopback_as_local() {
        use std::net::{Ipv4Addr, SocketAddr};
        assert!(is_loopback(None));
        assert!(is_loopback(Some(SocketAddr::new(
            Ipv4Addr::LOCALHOST.into(),
            7800
        ))));
        assert!(!is_loopback(Some(SocketAddr::new(
            Ipv4Addr::new(192, 168, 1, 5).into(),
            7800
        ))));
    }

    #[test]
    fn same_origin_and_headerless_requests_are_not_cross_site() {
        // CLI / pet: no Origin at all.
        assert!(!is_cross_site(None, Some("127.0.0.1:7800")));
        assert!(!is_cross_site(None, None));
        // The dashboard talking to its own server.
        assert!(!is_cross_site(
            Some("http://127.0.0.1:7800"),
            Some("127.0.0.1:7800")
        ));
        assert!(!is_cross_site(
            Some("http://localhost:7800"),
            Some("LOCALHOST:7800")
        ));
        assert!(!is_cross_site(
            Some("http://[::1]:7800"),
            Some("[::1]:7800")
        ));
        assert!(!is_cross_site(
            Some("http://192.168.1.5:7800"),
            Some("192.168.1.5:7800")
        ));
    }

    #[test]
    fn another_sites_page_is_cross_site() {
        assert!(is_cross_site(
            Some("https://evil.example"),
            Some("127.0.0.1:7800")
        ));
        // Same host, different port is a different origin.
        assert!(is_cross_site(
            Some("http://127.0.0.1:3000"),
            Some("127.0.0.1:7800")
        ));
        // Sandboxed iframes and file:// pages send the literal "null".
        assert!(is_cross_site(Some("null"), Some("127.0.0.1:7800")));
        assert!(is_cross_site(Some("https://evil.example"), None));
    }

    #[test]
    fn dns_rebinding_host_is_cross_site() {
        // A rebinding page's Origin matches its own Host, so the Host name is
        // what gives it away.
        assert!(is_cross_site(
            Some("http://evil.example:7800"),
            Some("evil.example:7800")
        ));
        assert!(is_cross_site(None, Some("evil.example:7800")));
    }

    #[test]
    fn trusted_loopback_requires_a_same_site_request() {
        use axum::http::HeaderMap;
        use std::net::Ipv4Addr;
        let local = Some(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 50000));
        let remote = Some(SocketAddr::new(Ipv4Addr::new(192, 168, 1, 9).into(), 50000));

        let mut own = HeaderMap::new();
        own.insert("host", "127.0.0.1:7800".parse().unwrap());
        assert!(trusted_loopback(local, &own));
        assert!(!trusted_loopback(remote, &own));

        let mut foreign = own.clone();
        foreign.insert("origin", "https://evil.example".parse().unwrap());
        assert!(!trusted_loopback(local, &foreign));
    }

    #[test]
    fn bearer_extracts_token() {
        assert_eq!(bearer(Some("Bearer abc123")), Some("abc123"));
    }

    #[test]
    fn bearer_rejects_malformed() {
        assert_eq!(bearer(None), None);
        assert_eq!(bearer(Some("abc123")), None); // missing scheme
        assert_eq!(bearer(Some("Bearer   ")), None); // empty token
    }
}
