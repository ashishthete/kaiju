//! Optional bearer-token authentication.
//!
//! When `NEXUS_TOKEN` is configured, every request to a protected route must
//! carry `Authorization: Bearer <token>`. With no token configured, auth is
//! disabled (the localhost-development default). The decision is a pure,
//! tested function; the middleware is a thin wrapper.

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::server::AppState;

/// Pure: is a request allowed, given the configured token and the bearer token
/// it presented? No configured token means auth is off (always allowed).
pub fn authorized(configured: &Option<String>, provided: Option<&str>) -> bool {
    match configured {
        None => true,
        Some(token) => provided == Some(token.as_str()),
    }
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

/// Middleware enforcing [`authorized`]. `/health` and `/` (the dashboard HTML)
/// are always public so liveness checks and the page load work unauthenticated.
pub async fn require_auth(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let path = req.uri().path();
    if path == "/health" || path == "/" {
        return next.run(req).await;
    }

    let header = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    if authorized(&state.auth_token, bearer(header)) {
        next.run(req).await
    } else {
        (StatusCode::UNAUTHORIZED, "unauthorized").into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_configured_token_allows_everything() {
        assert!(authorized(&None, None));
        assert!(authorized(&None, Some("anything")));
    }

    #[test]
    fn configured_token_requires_exact_match() {
        let configured = Some("secret".to_string());
        assert!(authorized(&configured, Some("secret")));
        assert!(!authorized(&configured, Some("wrong")));
        assert!(!authorized(&configured, None));
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
