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
        Some(p) => shared.as_deref() == Some(p) || device_tokens.iter().any(|t| t == p),
    }
}

/// True when the peer is loopback. `None` (no connection info — only happens in
/// the in-process test harness) is treated as loopback.
pub fn is_loopback(peer: Option<SocketAddr>) -> bool {
    peer.map_or(true, |addr| addr.ip().is_loopback())
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
        || path.ends_with("/terminal/ws")
    {
        return next.run(req).await;
    }

    let header = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());
    // Compute an owned token string before `req` is moved into `next.run`.
    let provided_owned: Option<String> = bearer(header).map(|s| s.to_string());

    let loopback = is_loopback(peer.map(|c| c.0));
    let tokens = state.devices.read().expect("devices lock").tokens();

    if authorized(loopback, &state.auth_token, &tokens, provided_owned.as_deref()) {
        // Refresh last-seen for the presenting device (no-op if not a device).
        if let Some(ref p) = provided_owned {
            state
                .devices
                .write()
                .expect("devices lock")
                .touch_by_token(p, chrono::Utc::now());
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
        assert!(!authorized(false, &Some("secret".into()), &[], Some("wrong")));
        assert!(!authorized(false, &None, &["dev-a".into()], Some("dev-b")));
    }

    #[test]
    fn remote_with_shared_token_is_allowed() {
        assert!(authorized(false, &Some("secret".into()), &[], Some("secret")));
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
        assert!(is_loopback(Some(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 7800))));
        assert!(!is_loopback(Some(SocketAddr::new(Ipv4Addr::new(192, 168, 1, 5).into(), 7800))));
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
