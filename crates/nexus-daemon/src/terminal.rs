//! In-browser interactive terminal: a WebSocket that mirrors an agent's tmux
//! pane (poll + repaint) and forwards keystrokes (raw `send-keys -H`).
//! Also serves the vendored xterm.js assets.

use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::server::AppState;
use crate::tmux::TmuxManager;

/// How often the pane is captured and (if changed) pushed to the browser.
const POLL_INTERVAL: Duration = Duration::from_millis(200);

const XTERM_JS: &str = include_str!("../assets/xterm.js");
const XTERM_CSS: &str = include_str!("../assets/xterm.css");

/// `GET /assets/xterm.js` — vendored renderer (public, no auth).
pub async fn xterm_js() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "application/javascript")], XTERM_JS)
}

/// `GET /assets/xterm.css` — vendored stylesheet (public, no auth).
pub async fn xterm_css() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css")], XTERM_CSS)
}

#[derive(Serialize)]
pub struct Size {
    cols: u16,
    rows: u16,
}

/// `GET /agents/:id/terminal/size` — pane dimensions so the browser sizes xterm
/// to match. Falls back to 80x24 if the agent/session can't be read.
pub async fn terminal_size(State(state): State<AppState>, Path(id): Path<String>) -> Json<Size> {
    let (cols, rows) = match state.store.get(&id) {
        Some(agent) => TmuxManager::pane_size(&agent.session_name).unwrap_or((80, 24)),
        None => (80, 24),
    };
    Json(Size { cols, rows })
}

#[derive(Deserialize)]
pub struct TokenQuery {
    /// Browsers can't set headers on a WS handshake, so auth rides the query.
    token: Option<String>,
}

/// `GET /agents/:id/terminal/ws` — upgrade to a terminal WebSocket.
///
/// Exempt from the header-based auth middleware; authenticates here against the
/// same configured token, taken from the query string.
pub async fn terminal_ws(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
) -> Response {
    if !crate::auth::authorized(&state.auth_token, q.token.as_deref()) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    let session = match state.store.get(&id) {
        Some(agent) => agent.session_name,
        None => return (StatusCode::NOT_FOUND, "agent not found").into_response(),
    };
    ws.on_upgrade(move |socket| run_terminal(socket, session))
}

/// Cheap content fingerprint, to skip resending unchanged frames.
fn fingerprint(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

/// Drive one terminal socket until it closes or the session ends.
///
/// One sequential loop (no split, no extra deps): capture+push a frame, then
/// wait up to `POLL_INTERVAL` for a keystroke. A keystroke loops immediately so
/// the result is reflected promptly; a timeout just paces the next frame.
async fn run_terminal(mut socket: WebSocket, session: String) {
    let mut last: u64 = 0;
    loop {
        // 1. Capture the pane (blocking tmux call off the async thread) and
        //    push a repaint frame if it changed.
        let s = session.clone();
        match tokio::task::spawn_blocking(move || TmuxManager::capture_pane_colored(&s)).await {
            Ok(Ok(frame)) => {
                let fp = fingerprint(&frame);
                if fp != last {
                    last = fp;
                    // Home cursor + clear, then the screen: a stable repaint.
                    let payload = format!("\x1b[H\x1b[J{frame}");
                    if socket.send(Message::Text(payload)).await.is_err() {
                        return;
                    }
                }
            }
            _ => {
                let _ = socket
                    .send(Message::Text("\r\n[session ended]\r\n".to_string()))
                    .await;
                return;
            }
        }

        // 2. Wait briefly for input; forward raw bytes to tmux.
        match tokio::time::timeout(POLL_INTERVAL, socket.recv()).await {
            Ok(Some(Ok(Message::Text(t)))) => forward(&session, t.into_bytes()).await,
            Ok(Some(Ok(Message::Binary(b)))) => forward(&session, b).await,
            Ok(Some(Ok(Message::Close(_)))) | Ok(None) => return,
            Ok(Some(Ok(_))) => {}     // ping/pong/other — ignore
            Ok(Some(Err(_))) => return, // socket error
            Err(_elapsed) => {}        // no input this tick — refresh
        }
    }
}

async fn forward(session: &str, bytes: Vec<u8>) {
    // Defense-in-depth: ignore absurdly large input frames (an authenticated
    // localhost tool, but no reason to hex-expand megabytes into argv).
    const MAX_INPUT: usize = 64 * 1024;
    if bytes.is_empty() || bytes.len() > MAX_INPUT {
        return;
    }
    let s = session.to_string();
    let _ = tokio::task::spawn_blocking(move || TmuxManager::send_raw_bytes(&s, &bytes)).await;
}
