//! Device pairing endpoints. A trusted device (loopback, or already paired)
//! mints a one-time code + QR; a new device redeems the code for its own token.

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::server::AppState;

#[derive(Serialize)]
pub struct PairCodeResponse {
    pub code: String,
    pub url: String,
    pub qr_svg: String,
}

/// `POST /pair/code` — issue a one-time code with its pairing URL + QR. Behind
/// the auth middleware, so only a trusted (loopback or paired) caller reaches it.
pub async fn pair_code(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let code = crate::pairing::generate_code();
    state
        .pending_codes
        .lock()
        .expect("codes lock")
        .issue(code.clone(), chrono::Utc::now());

    let host = advertised_base(&headers);
    let url = format!("{host}/pair?code={code}");
    let qr_svg = crate::pairing::qr_svg(&url).unwrap_or_else(|e| {
        tracing::warn!("QR generation failed for pairing URL: {e}");
        String::new()
    });
    Json(PairCodeResponse { code, url, qr_svg })
}

/// Build the base URL (`http://host:port`) to advertise. Prefers the detected
/// LAN IP; otherwise reuses the Host header the browser already reached us on.
fn advertised_base(headers: &axum::http::HeaderMap) -> String {
    let port = std::env::var("KAIJU_PORT").unwrap_or_else(|_| "7800".to_string());
    let bind_host = std::env::var("KAIJU_HOST").ok();
    if let Some(ip) = crate::net::advertised_host(bind_host.as_deref()) {
        return format!("http://{ip}:{port}");
    }
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("127.0.0.1:7800");
    format!("http://{host}")
}

#[derive(Deserialize)]
pub struct ClaimRequest {
    pub code: String,
    pub name: Option<String>,
}

#[derive(Serialize)]
pub struct ClaimResponse {
    pub token: String,
}

/// `POST /pair/claim` — redeem a code for a fresh device token. Public (an
/// unpaired device must reach it). Returns 403 for an unknown/expired code.
pub async fn pair_claim(
    State(state): State<AppState>,
    Json(req): Json<ClaimRequest>,
) -> impl IntoResponse {
    let now = chrono::Utc::now();
    let ok = state
        .pending_codes
        .lock()
        .expect("codes lock")
        .redeem(&req.code, now);
    if !ok {
        return Err((
            StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({ "error": "invalid or expired code" })),
        ));
    }

    let token = uuid::Uuid::new_v4().to_string();
    let name = req
        .name
        .map(|n| n.trim().chars().take(64).collect::<String>())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "device".to_string());
    {
        let mut devices = state.devices.write().expect("devices lock");
        devices.add(name, token.clone(), now);
        // Best-effort persist; a failed write still leaves the device usable
        // until restart.
        let _ = crate::devices::save(&devices);
    }
    Ok(Json(ClaimResponse { token }))
}

/// `GET /pair` — the tiny claim page a scanned QR opens. Public.
pub async fn pair_page() -> Html<&'static str> {
    Html(crate::dashboard::PAIR_PAGE)
}

#[derive(Serialize)]
pub struct DeviceRow {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub last_seen: String,
}

/// `GET /devices` — list paired devices (no tokens). Behind auth.
pub async fn list_devices(State(state): State<AppState>) -> impl IntoResponse {
    let devices = state.devices.read().expect("devices lock");
    let rows: Vec<DeviceRow> = devices
        .devices
        .iter()
        .map(|d| DeviceRow {
            id: d.id.clone(),
            name: d.name.clone(),
            created_at: d.created_at.to_rfc3339(),
            last_seen: d.last_seen.to_rfc3339(),
        })
        .collect();
    Json(rows)
}

/// `DELETE /devices/:id` — revoke a device. Behind auth.
pub async fn revoke_device(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let removed = {
        let mut devices = state.devices.write().expect("devices lock");
        let removed = devices.remove(&id);
        if removed {
            let _ = crate::devices::save(&devices);
        }
        removed
    };
    if removed {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}
