# LAN Exposure + Device Pairing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the Kaiju daemon listen on the LAN (`0.0.0.0`) safely, where the host machine is the trust anchor and additional devices are paired via a one-time code shown as a QR in the dashboard, each getting its own revocable token.

**Architecture:** A new `KAIJU_HOST` env var controls the bind address (default stays `127.0.0.1`). Authorization is reworked into one pure predicate: loopback peers are always trusted (the host + the in-process test harness), and remote peers must present either the legacy shared `KAIJU_TOKEN` or a registered per-device token. Per-device tokens live in `~/.kaiju/devices.json`. A trusted device generates a short-lived one-time pairing code; the dashboard renders it as a QR pointing at `http://<lan-ip>:<port>/pair?code=...`. A new device opens that URL, claims the code, receives its own token, and stores it in `localStorage` exactly like today's shared token (header `Authorization: Bearer` + WS `?token=`).

**Tech Stack:** Rust, Axum (with `ConnectInfo` for peer IP), Tokio, `uuid` (token/code generation — already a dep), `chrono` (expiry — already a dep), the `qrcode` crate (server-side SVG), vanilla JS dashboard.

---

## File Structure

- `crates/kaiju-daemon/src/main.rs` — **modify**: parse `KAIJU_HOST` into the bind `SocketAddr`.
- `crates/kaiju-daemon/src/net.rs` — **create**: loopback test + LAN-IP detection (pure-ish helpers).
- `crates/kaiju-daemon/src/auth.rs` — **modify**: rewrite `authorized` to the loopback/shared/device predicate; middleware gains peer IP + device tokens + `last_seen` touch.
- `crates/kaiju-daemon/src/devices.rs` — **create**: `Device`, `Devices` store, load/save (mirrors `settings.rs`).
- `crates/kaiju-daemon/src/pairing.rs` — **create**: one-time pairing codes (generate, validate, consume, expire) + QR SVG.
- `crates/kaiju-daemon/src/server.rs` — **modify**: add `devices` + `pending_codes` to `AppState`; load devices at startup; switch `run()` to `into_make_service_with_connect_info`.
- `crates/kaiju-daemon/src/api.rs` — **modify**: register pairing + device routes.
- `crates/kaiju-daemon/src/pair_api.rs` — **create**: handlers for `/pair/code`, `/pair` (page), `/pair/claim`, `/devices`, `/devices/:id`.
- `crates/kaiju-daemon/src/dashboard.rs` — **modify**: Devices section in the Preferences popover; the `/pair` claim page HTML.
- `crates/kaiju-daemon/assets/dashboard.js` — **modify**: pair-a-device flow (fetch code, show QR), device list + revoke.
- `crates/kaiju-daemon/src/lib.rs` — **modify**: declare new modules.
- `crates/kaiju-daemon/Cargo.toml` + root `Cargo.toml` — **modify**: add the `qrcode` dependency.
- `crates/kaiju-daemon/tests/api.rs` — **modify**: integration tests for pairing + auth.

**Natural checkpoint:** Task 1 (bind config) is independently shippable and useful on its own. Tasks 2–10 build the pairing system. Consider committing/PRing Task 1 separately.

---

### Task 1: `KAIJU_HOST` bind configuration

**Files:**
- Create: `crates/kaiju-daemon/src/net.rs` (the parse helper lands here so it's unit-testable)
- Modify: `crates/kaiju-daemon/src/main.rs:14-19`
- Modify: `crates/kaiju-daemon/src/lib.rs` (declare `pub mod net;`)

- [ ] **Step 1: Write the failing test**

Create `crates/kaiju-daemon/src/net.rs`:

```rust
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
        assert_eq!(bind_ip(Some("192.168.1.5")), "192.168.1.5".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn garbage_falls_back_to_loopback() {
        assert_eq!(bind_ip(Some("not-an-ip")), IpAddr::V4(Ipv4Addr::LOCALHOST));
    }
}
```

Add to `crates/kaiju-daemon/src/lib.rs` (with the other `pub mod` lines):

```rust
pub mod net;
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kaiju-daemon net::tests`
Expected: FAIL to compile / module not found until `net.rs` is wired — then PASS once added. (If it already passes after adding the file, that's fine; the point is the test exists and exercises `bind_ip`.)

- [ ] **Step 3: Wire the bind address in `main.rs`**

Replace `crates/kaiju-daemon/src/main.rs:14-19` (the port parse + `addr` line) with:

```rust
    let port: u16 = std::env::var("KAIJU_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(7800);

    let host = std::env::var("KAIJU_HOST").ok();
    let addr = SocketAddr::new(kaiju_daemon::net::bind_ip(host.as_deref()), port);

    if host.as_deref().map(|h| h != "127.0.0.1").unwrap_or(false) {
        tracing::warn!(
            "listening on {addr} — reachable from your local network. \
             Set KAIJU_TOKEN or pair devices; unpaired remote requests are rejected."
        );
    }
```

(Keep the existing `use kaiju_daemon::server;` and `use std::net::SocketAddr;` at the top, and the `server::run(addr)` call below unchanged.)

- [ ] **Step 4: Run tests + build to verify**

Run: `cargo test -p kaiju-daemon net::tests && cargo build -p kaiju-daemon`
Expected: tests PASS, build succeeds.

- [ ] **Step 5: Commit**

```bash
git add crates/kaiju-daemon/src/net.rs crates/kaiju-daemon/src/lib.rs crates/kaiju-daemon/src/main.rs
git commit -m "feat(daemon): KAIJU_HOST to choose the bind address"
```

---

### Task 2: LAN-IP detection for the pairing URL

**Files:**
- Modify: `crates/kaiju-daemon/src/net.rs`

- [ ] **Step 1: Write the failing test**

Append to the `tests` module in `crates/kaiju-daemon/src/net.rs`:

```rust
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kaiju-daemon net::tests::advertised_host`
Expected: FAIL — `advertised_host` not defined.

- [ ] **Step 3: Implement `advertised_host` + `detect_lan_ip`**

Add to `crates/kaiju-daemon/src/net.rs` (above the `tests` module):

```rust
use std::net::UdpSocket;

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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p kaiju-daemon net::tests`
Expected: PASS. (`advertised_host_ignores_loopback_and_unspecified` passes whether detection returns an IP or `None`, since either way the result isn't the loopback/unspecified string.)

- [ ] **Step 5: Commit**

```bash
git add crates/kaiju-daemon/src/net.rs
git commit -m "feat(daemon): detect the LAN IP to advertise for pairing"
```

---

### Task 3: Authorization predicate (loopback / shared / device)

**Files:**
- Modify: `crates/kaiju-daemon/src/auth.rs:15-33` (rewrite `authorized`, add `is_loopback`)
- Test: same file's `#[cfg(test)] mod tests`

- [ ] **Step 1: Write the failing test**

Replace the existing `no_configured_token_allows_everything` and `configured_token_requires_exact_match` tests in `crates/kaiju-daemon/src/auth.rs` with:

```rust
    #[test]
    fn loopback_is_always_trusted() {
        // The host machine (and the in-process test harness) need no token.
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
        assert!(is_loopback(None)); // in-process (no socket) = local
        assert!(is_loopback(Some(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 7800))));
        assert!(!is_loopback(Some(SocketAddr::new(Ipv4Addr::new(192, 168, 1, 5).into(), 7800))));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kaiju-daemon auth::`
Expected: FAIL — `authorized` arity changed / `is_loopback` not defined.

- [ ] **Step 3: Rewrite `authorized` and add `is_loopback`**

Replace `crates/kaiju-daemon/src/auth.rs:15-22` (the old `authorized`) with:

```rust
use std::net::SocketAddr;

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
            shared.as_deref() == Some(p) || device_tokens.iter().any(|t| t == p)
        }
    }
}

/// True when the peer is loopback. `None` (no connection info — only happens in
/// the in-process test harness) is treated as loopback.
pub fn is_loopback(peer: Option<SocketAddr>) -> bool {
    peer.map_or(true, |addr| addr.ip().is_loopback())
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p kaiju-daemon auth::`
Expected: PASS. (The middleware `require_auth` won't compile yet — it still calls the old signature. That's fixed in Task 6. To keep this task's commit green, temporarily leave `require_auth` as-is only if it still compiles; if not, proceed to Step 5 knowing Task 6 completes the wiring. Prefer running just the `auth::tests` here: `cargo test -p kaiju-daemon auth::tests` compiles the lib — so do Step 5's middleware shim now.)

- [ ] **Step 5: Update `require_auth` to the new predicate (minimal shim)**

Replace the body of `require_auth` in `crates/kaiju-daemon/src/auth.rs` (the `let header = ...; if authorized(...)` portion, lines ~50-59) with the loopback-aware version. Add the `ConnectInfo` extractor to its signature:

```rust
use axum::extract::ConnectInfo;
use std::net::SocketAddr;

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

    let provided = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|h| bearer(Some(h)));

    let loopback = is_loopback(peer.map(|c| c.0));
    let tokens = state.devices.read().expect("devices lock").tokens();

    if authorized(loopback, &state.auth_token, &tokens, provided) {
        // Refresh last-seen for the presenting device (no-op if not a device).
        if let Some(p) = provided {
            state.devices.write().expect("devices lock").touch_by_token(p);
        }
        next.run(req).await
    } else {
        (StatusCode::UNAUTHORIZED, "unauthorized").into_response()
    }
}
```

> NOTE: `state.devices` (an `Arc<RwLock<Devices>>` with `.tokens()` and `.touch_by_token()`) is added in Tasks 4 and 6. If implementing strictly in order, this step won't compile until Task 6 adds the `devices` field. Acceptable approaches: (a) do Tasks 4 + 6 immediately after this, committing them together; or (b) recommended — implement Tasks 4, 6 before compiling/committing Task 3's middleware. The pure `authorized`/`is_loopback` functions and their tests (Steps 1–4) stand alone and can be committed first.

- [ ] **Step 6: Commit (pure predicate first)**

```bash
git add crates/kaiju-daemon/src/auth.rs
git commit -m "feat(daemon): loopback-trust + device-token authorization predicate"
```

---

### Task 4: Device store (`devices.json`)

**Files:**
- Create: `crates/kaiju-daemon/src/devices.rs`
- Modify: `crates/kaiju-daemon/src/lib.rs` (declare `pub mod devices;`)

- [ ] **Step 1: Write the failing test**

Create `crates/kaiju-daemon/src/devices.rs`:

```rust
//! Per-device pairing tokens, persisted to `~/.kaiju/devices.json`
//! (override with `KAIJU_DEVICES`). Mirrors `settings.rs`: optional file,
//! best-effort load, explicit save. Each paired device holds its own token in
//! its browser; revoking a device deletes its row here.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use kaiju_core::{NexusError, Result};
use serde::{Deserialize, Serialize};

/// One paired device. `token` is the secret it presents (bearer / WS query).
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub token: String,
    pub created_at: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}

/// The set of paired devices.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Devices {
    #[serde(default)]
    pub devices: Vec<Device>,
}

impl Devices {
    /// All device tokens (for the auth check).
    pub fn tokens(&self) -> Vec<String> {
        self.devices.iter().map(|d| d.token.clone()).collect()
    }

    /// Add a device. Returns the created row.
    pub fn add(&mut self, name: String, token: String, now: DateTime<Utc>) -> Device {
        let device = Device {
            id: uuid::Uuid::new_v4().simple().to_string(),
            name,
            token,
            created_at: now,
            last_seen: now,
        };
        self.devices.push(device.clone());
        device
    }

    /// Remove a device by id. Returns true if one was removed.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.devices.len();
        self.devices.retain(|d| d.id != id);
        self.devices.len() != before
    }

    /// Update `last_seen` for whichever device presented this token (if any).
    pub fn touch_by_token(&mut self, token: &str) {
        if let Some(d) = self.devices.iter_mut().find(|d| d.token == token) {
            d.last_seen = Utc::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        "2026-06-11T00:00:00Z".parse().unwrap()
    }

    #[test]
    fn add_then_token_is_listed() {
        let mut d = Devices::default();
        let row = d.add("Phone".into(), "tok-1".into(), now());
        assert_eq!(d.tokens(), vec!["tok-1".to_string()]);
        assert_eq!(row.name, "Phone");
        assert!(!row.id.is_empty());
    }

    #[test]
    fn remove_by_id() {
        let mut d = Devices::default();
        let row = d.add("Phone".into(), "tok-1".into(), now());
        assert!(d.remove(&row.id));
        assert!(d.tokens().is_empty());
        assert!(!d.remove("nope"));
    }

    #[test]
    fn touch_only_matching_token() {
        let mut d = Devices::default();
        d.add("Phone".into(), "tok-1".into(), now());
        d.touch_by_token("tok-1"); // no panic, updates in place
        d.touch_by_token("unknown"); // no-op
        assert_eq!(d.devices.len(), 1);
    }
}
```

Add to `crates/kaiju-daemon/src/lib.rs`:

```rust
pub mod devices;
```

- [ ] **Step 2: Run test to verify it fails, then passes**

Run: `cargo test -p kaiju-daemon devices::`
Expected: PASS once the file + module declaration are in place.

- [ ] **Step 3: Add the load/save free functions**

Append to `crates/kaiju-daemon/src/devices.rs` (above `#[cfg(test)]`):

```rust
/// Path to the devices file: `KAIJU_DEVICES`, else `~/.kaiju/devices.json`.
fn devices_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("KAIJU_DEVICES") {
        return Some(PathBuf::from(path));
    }
    std::env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join(".kaiju").join("devices.json"))
}

/// Read paired devices; an absent or malformed file means "no devices".
pub fn load() -> Devices {
    let Some(path) = devices_path() else {
        return Devices::default();
    };
    let Ok(content) = std::fs::read_to_string(path) else {
        return Devices::default();
    };
    serde_json::from_str(&content).unwrap_or_default()
}

/// Persist devices (pretty JSON), creating the directory if needed.
pub fn save(devices: &Devices) -> Result<()> {
    let path = devices_path().ok_or_else(|| {
        NexusError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no devices path (set KAIJU_DEVICES or HOME)",
        ))
    })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(NexusError::Io)?;
    }
    let json = serde_json::to_string_pretty(devices)
        .map_err(|e| NexusError::Io(std::io::Error::other(e.to_string())))?;
    std::fs::write(&path, json).map_err(NexusError::Io)?;
    Ok(())
}
```

- [ ] **Step 4: Run tests to verify**

Run: `cargo test -p kaiju-daemon devices::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/kaiju-daemon/src/devices.rs crates/kaiju-daemon/src/lib.rs
git commit -m "feat(daemon): per-device token store (devices.json)"
```

---

### Task 5: One-time pairing codes + QR

**Files:**
- Modify: root `Cargo.toml` and `crates/kaiju-daemon/Cargo.toml` (add `qrcode`)
- Create: `crates/kaiju-daemon/src/pairing.rs`
- Modify: `crates/kaiju-daemon/src/lib.rs` (declare `pub mod pairing;`)

- [ ] **Step 1: Add the `qrcode` dependency**

In root `Cargo.toml`, under `[workspace.dependencies]`, add:

```toml
qrcode = "0.14"
```

In `crates/kaiju-daemon/Cargo.toml`, under `[dependencies]`, add:

```toml
qrcode = { workspace = true }
```

- [ ] **Step 2: Write the failing test**

Create `crates/kaiju-daemon/src/pairing.rs`:

```rust
//! Short-lived, single-use pairing codes. A trusted device asks for a code; a
//! new device redeems it once, within the TTL, to receive its own token. Codes
//! live only in memory — losing them on restart is fine (just re-pair).

use chrono::{DateTime, Duration, Utc};

/// How long a freshly issued code stays redeemable.
pub const CODE_TTL_MINUTES: i64 = 10;

/// A pending, not-yet-redeemed pairing code.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingCode {
    pub code: String,
    pub expires_at: DateTime<Utc>,
}

/// Generate a human-typeable 8-char uppercase code (UUID-derived).
pub fn generate_code() -> String {
    uuid::Uuid::new_v4()
        .simple()
        .to_string()
        .chars()
        .take(8)
        .collect::<String>()
        .to_uppercase()
}

/// In-memory set of outstanding codes.
#[derive(Debug, Default)]
pub struct PendingCodes {
    codes: Vec<PendingCode>,
}

impl PendingCodes {
    /// Issue a code valid until `now + CODE_TTL_MINUTES`.
    pub fn issue(&mut self, code: String, now: DateTime<Utc>) {
        self.prune(now);
        self.codes.push(PendingCode {
            code,
            expires_at: now + Duration::minutes(CODE_TTL_MINUTES),
        });
    }

    /// Redeem a code: valid + unexpired removes it and returns true (single
    /// use). Unknown or expired returns false.
    pub fn redeem(&mut self, code: &str, now: DateTime<Utc>) -> bool {
        self.prune(now);
        let before = self.codes.len();
        self.codes.retain(|c| c.code != code);
        self.codes.len() != before
    }

    /// Drop expired codes.
    fn prune(&mut self, now: DateTime<Utc>) {
        self.codes.retain(|c| c.expires_at > now);
    }
}

/// Render a pairing URL as an inline SVG QR code.
pub fn qr_svg(url: &str) -> Result<String, qrcode::types::QrError> {
    use qrcode::render::svg;
    use qrcode::QrCode;
    let code = QrCode::new(url.as_bytes())?;
    Ok(code
        .render::<svg::Color>()
        .min_dimensions(220, 220)
        .quiet_zone(true)
        .build())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[test]
    fn generated_code_is_eight_upper_chars() {
        let c = generate_code();
        assert_eq!(c.len(), 8);
        assert_eq!(c, c.to_uppercase());
    }

    #[test]
    fn redeem_is_single_use() {
        let mut codes = PendingCodes::default();
        let now = at("2026-06-11T00:00:00Z");
        codes.issue("ABC123XY".into(), now);
        assert!(codes.redeem("ABC123XY", now));
        assert!(!codes.redeem("ABC123XY", now)); // already consumed
    }

    #[test]
    fn expired_code_is_rejected() {
        let mut codes = PendingCodes::default();
        codes.issue("ABC123XY".into(), at("2026-06-11T00:00:00Z"));
        // 11 minutes later — past the 10-minute TTL.
        assert!(!codes.redeem("ABC123XY", at("2026-06-11T00:11:00Z")));
    }

    #[test]
    fn unknown_code_is_rejected() {
        let mut codes = PendingCodes::default();
        assert!(!codes.redeem("NOPE0000", at("2026-06-11T00:00:00Z")));
    }

    #[test]
    fn qr_svg_renders_svg_for_url() {
        let svg = qr_svg("http://192.168.1.5:7800/pair?code=ABC123XY").unwrap();
        assert!(svg.contains("<svg"));
    }
}
```

Add to `crates/kaiju-daemon/src/lib.rs`:

```rust
pub mod pairing;
```

- [ ] **Step 3: Run test to verify it passes**

Run: `cargo test -p kaiju-daemon pairing::`
Expected: PASS. (If `qrcode 0.14`'s `QrError` path differs, adjust the `qr_svg` return type to the error type the compiler reports — the test only asserts the success branch.)

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml crates/kaiju-daemon/Cargo.toml crates/kaiju-daemon/src/pairing.rs crates/kaiju-daemon/src/lib.rs
git commit -m "feat(daemon): one-time pairing codes and QR rendering"
```

---

### Task 6: Wire `AppState`, startup, and the connect-info service

**Files:**
- Modify: `crates/kaiju-daemon/src/server.rs:26-62` (`AppState` + constructors)
- Modify: `crates/kaiju-daemon/src/server.rs:174-205` (`run`: load devices, connect-info service)

- [ ] **Step 1: Add fields to `AppState`**

In `crates/kaiju-daemon/src/server.rs`, extend the struct (after the `settings` field, line ~36):

```rust
    /// Paired devices and their tokens. Mutable at runtime (pair / revoke).
    pub devices: std::sync::Arc<std::sync::RwLock<crate::devices::Devices>>,
    /// Outstanding one-time pairing codes (in-memory; lost on restart).
    pub pending_codes: std::sync::Arc<std::sync::Mutex<crate::pairing::PendingCodes>>,
```

In `with_stores` (line ~51), add the initializers (after the `settings` initializer):

```rust
            devices: std::sync::Arc::new(std::sync::RwLock::new(
                crate::devices::Devices::default(),
            )),
            pending_codes: std::sync::Arc::new(std::sync::Mutex::new(
                crate::pairing::PendingCodes::default(),
            )),
```

- [ ] **Step 2: Load devices at startup and use a connect-info service**

In `run` (`crates/kaiju-daemon/src/server.rs`), after the `state.settings = ...` line (~185), add:

```rust
    state.devices = std::sync::Arc::new(std::sync::RwLock::new(crate::devices::load()));
```

Then replace the serve line (`axum::serve(listener, app)...`, ~202) with the connect-info variant so the auth middleware receives peer addresses:

```rust
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .map_err(NexusError::Io)?;
```

- [ ] **Step 3: Verify the whole crate compiles with the new middleware**

Run: `cargo build -p kaiju-daemon`
Expected: PASS. (`require_auth` from Task 3 now resolves `state.devices.tokens()` / `touch_by_token`.)

- [ ] **Step 4: Migrate the existing auth integration tests to simulate a remote peer**

The new model trusts loopback peers unconditionally, and `is_loopback(None)` is `true` — so the in-process `oneshot` harness counts as loopback. That keeps the *non-auth* tests green (they hit protected routes with no token and expect success — now allowed as loopback). But the three existing **auth** tests that assert token *enforcement* must now simulate a **remote** peer, otherwise loopback-trust bypasses the token and they'd wrongly pass/fail.

In `crates/kaiju-daemon/tests/api.rs`, add a helper that attaches a non-loopback `ConnectInfo` to a request (the same extension `into_make_service_with_connect_info` inserts in production), near the other request helpers at the top:

```rust
use axum::extract::ConnectInfo;
use std::net::SocketAddr;

/// A request carrying a simulated remote (non-loopback) peer, so the auth
/// middleware exercises token enforcement rather than loopback trust.
fn remote_request(uri: &str) -> Request<Body> {
    let mut req = Request::builder().uri(uri).body(Body::empty()).unwrap();
    req.extensions_mut().insert(ConnectInfo(
        "203.0.113.7:54321".parse::<SocketAddr>().unwrap(),
    ));
    req
}

/// A remote request that also presents a bearer token.
fn remote_request_with_token(uri: &str, token: &str) -> Request<Body> {
    let mut req = remote_request(uri);
    req.headers_mut().insert(
        "authorization",
        format!("Bearer {token}").parse().unwrap(),
    );
    req
}
```

Then update the three auth tests to use these helpers:

```rust
#[tokio::test]
async fn protected_route_rejects_missing_token() {
    // Remote peer, token configured, no header → rejected.
    let resp = build_app(authed_state())
        .oneshot(remote_request("/agents"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn protected_route_accepts_valid_token() {
    let resp = build_app(authed_state())
        .oneshot(remote_request_with_token("/agents", "secret"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn loopback_peer_is_trusted_without_token() {
    // No ConnectInfo (in-process) = loopback = always allowed, even with a
    // token configured. This is the host-machine trust anchor.
    let resp = build_app(authed_state())
        .oneshot(get_request("/agents"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn remote_peer_rejected_when_no_token_configured() {
    // The LAN-exposed-without-a-shared-token case: a remote, unpaired device
    // is still rejected (it must pair first).
    let resp = build_app(AppState::new())
        .oneshot(remote_request("/agents"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
```

(`health_and_dashboard_are_public_under_auth` is unchanged — those routes are public before the auth check, so loopback-vs-remote is irrelevant.)

- [ ] **Step 5: Run the full integration + lib suite (regression check)**

Run: `cargo test -p kaiju-daemon`
Expected: PASS — non-auth `oneshot` tests stay green (loopback trust), and the migrated auth tests now exercise the remote/token matrix.

- [ ] **Step 6: Commit**

```bash
git add crates/kaiju-daemon/src/server.rs crates/kaiju-daemon/src/auth.rs crates/kaiju-daemon/tests/api.rs
git commit -m "feat(daemon): wire devices + pairing into AppState and the server"
```

---

### Task 7: Pairing + device HTTP endpoints

**Files:**
- Create: `crates/kaiju-daemon/src/pair_api.rs`
- Modify: `crates/kaiju-daemon/src/lib.rs` (declare `pub mod pair_api;`)
- Modify: `crates/kaiju-daemon/src/api.rs:15-50` (register routes)

- [ ] **Step 1: Write the failing integration test**

Append to `crates/kaiju-daemon/tests/api.rs`:

```rust
#[tokio::test]
async fn pair_claim_with_valid_code_returns_a_token() {
    let state = AppState::new();
    // Issue a code the way the trusted endpoint would.
    let now = chrono::Utc::now();
    state
        .pending_codes
        .lock()
        .unwrap()
        .issue("TESTCODE".into(), now);
    let app = build_app(state.clone());

    let resp = app
        .oneshot(json_request(
            "POST",
            "/pair/claim",
            serde_json::json!({ "code": "TESTCODE", "name": "Phone" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert!(json["token"].as_str().is_some());
    // The device is now registered.
    assert_eq!(state.devices.read().unwrap().devices.len(), 1);
}

#[tokio::test]
async fn pair_claim_with_bad_code_is_rejected() {
    let app = build_app(AppState::new());
    let resp = app
        .oneshot(json_request(
            "POST",
            "/pair/claim",
            serde_json::json!({ "code": "WRONG", "name": "Phone" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kaiju-daemon --test api pair_claim`
Expected: FAIL — `/pair/claim` route returns 404 (not registered yet).

- [ ] **Step 3: Implement the handlers**

Create `crates/kaiju-daemon/src/pair_api.rs`:

```rust
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
/// `host` is the advertised LAN host; falls back to the request's Host header.
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
    let qr_svg = match crate::pairing::qr_svg(&url) {
        Ok(svg) => svg,
        Err(_) => String::new(),
    };
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
        return Err((StatusCode::FORBIDDEN, "invalid or expired code"));
    }

    let token = uuid::Uuid::new_v4().simple().to_string();
    let name = req.name.unwrap_or_else(|| "device".to_string());
    {
        let mut devices = state.devices.write().expect("devices lock");
        devices.add(name, token.clone(), now);
        // Best-effort persist; a failed write still leaves the device usable
        // until restart.
        let _ = crate::devices::save(&devices);
    }
    Ok(Json(ClaimResponse { token }))
}

/// `GET /pair` — the tiny claim page a scanned QR opens. Public. It reads
/// `?code=` from its own URL, POSTs to `/pair/claim`, stores the returned token
/// in localStorage (same key the dashboard uses), then redirects to `/`.
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
```

Add to `crates/kaiju-daemon/src/lib.rs`:

```rust
pub mod pair_api;
```

- [ ] **Step 4: Register the routes**

In `crates/kaiju-daemon/src/api.rs`, inside `routes()` (after the `/settings` route, line ~45), add:

```rust
        .route("/pair", get(crate::pair_api::pair_page))
        .route("/pair/code", post(crate::pair_api::pair_code))
        .route("/pair/claim", post(crate::pair_api::pair_claim))
        .route("/devices", get(crate::pair_api::list_devices))
        .route("/devices/:id", axum::routing::delete(crate::pair_api::revoke_device))
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p kaiju-daemon --test api pair_claim`
Expected: PASS (both `pair_claim` tests).

- [ ] **Step 6: Commit**

```bash
git add crates/kaiju-daemon/src/pair_api.rs crates/kaiju-daemon/src/lib.rs crates/kaiju-daemon/src/api.rs crates/kaiju-daemon/tests/api.rs
git commit -m "feat(daemon): pairing + device-management endpoints"
```

---

### Task 8: The `/pair` claim page

**Files:**
- Modify: `crates/kaiju-daemon/src/dashboard.rs` (add `PAIR_PAGE`)

- [ ] **Step 1: Add the claim page constant**

In `crates/kaiju-daemon/src/dashboard.rs`, add (near `PAGE`, after it):

```rust
/// The pairing claim page served at `GET /pair`. A scanned QR lands here with
/// `?code=...`; it redeems the code and saves the returned token under the same
/// `kaiju_token` localStorage key the dashboard reads, then redirects to `/`.
pub const PAIR_PAGE: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Pair this device — Kaiju</title>
<style>
  body { font: 16px system-ui, sans-serif; margin: 0; display: grid; place-items: center;
         min-height: 100vh; background: #0b0c0f; color: #e6e6e6; }
  .card { max-width: 360px; padding: 28px; text-align: center; }
  h1 { font-size: 20px; margin: 0 0 12px; }
  input { width: 100%; padding: 10px; margin: 10px 0; box-sizing: border-box;
          border-radius: 8px; border: 1px solid #333; background: #15171c; color: #e6e6e6; }
  button { padding: 10px 18px; border-radius: 8px; border: 0; background: #5b8cff;
           color: #fff; font-weight: 600; cursor: pointer; }
  .msg { margin-top: 12px; min-height: 1.4em; color: #ff8080; }
</style>
</head>
<body>
  <div class="card">
    <h1>Pair this device</h1>
    <p>Name this device, then pair to access the Kaiju dashboard.</p>
    <input id="name" placeholder="e.g. My phone" autocomplete="off">
    <button onclick="claim()">Pair</button>
    <div class="msg" id="msg"></div>
  </div>
<script>
  const params = new URLSearchParams(location.search);
  const code = params.get("code") || "";
  document.getElementById("name").value =
    /iphone|android|ipad|mobile/i.test(navigator.userAgent) ? "Phone" : "";
  async function claim() {
    const name = document.getElementById("name").value || "device";
    const msg = document.getElementById("msg");
    if (!code) { msg.textContent = "Missing pairing code in the link."; return; }
    msg.textContent = "Pairing…";
    try {
      const res = await fetch("/pair/claim", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ code, name }),
      });
      if (!res.ok) { msg.textContent = "Code invalid or expired. Ask for a new one."; return; }
      const data = await res.json();
      localStorage.setItem("kaiju_token", data.token);
      location.href = "/";
    } catch (e) {
      msg.textContent = "Network error. Are you on the same network?";
    }
  }
</script>
</body>
</html>"#;
```

- [ ] **Step 2: Verify it compiles and serves**

Run: `cargo build -p kaiju-daemon`
Expected: PASS. (`pair_page()` from Task 7 references `crate::dashboard::PAIR_PAGE`, now defined.)

- [ ] **Step 3: Commit**

```bash
git add crates/kaiju-daemon/src/dashboard.rs
git commit -m "feat(dashboard): pairing claim page served at /pair"
```

---

### Task 9: Dashboard "Devices" UI (pair + revoke)

**Files:**
- Modify: `crates/kaiju-daemon/src/dashboard.rs:218-255` (Devices section in the Preferences popover)
- Modify: `crates/kaiju-daemon/assets/dashboard.js` (pair flow + device list)

- [ ] **Step 1: Add the Devices section to the Preferences popover**

In `crates/kaiju-daemon/src/dashboard.rs`, inside the `#settings-pop` popover (before the closing `</div>` at line ~255, after the last `.pop-hint`), add:

```html
    <div class="pop-section">Devices</div>
    <div class="pop-hint">Pair another device on your network. The host machine is always trusted.</div>
    <div id="device-list" class="device-list"></div>
    <div class="pop-actions"><button onclick="startPairing()">Pair a device</button></div>
    <div id="pair-box" hidden style="text-align:center;margin-top:10px">
      <div id="pair-qr"></div>
      <div class="pop-hint">Scan this, or open <code id="pair-url"></code> and enter
        <strong id="pair-code"></strong>. Valid for 10 minutes.</div>
    </div>
```

- [ ] **Step 2: Add the device-list + pairing JS**

Append to `crates/kaiju-daemon/assets/dashboard.js`:

```javascript
// --- Device pairing ---

async function loadDevices() {
  const box = document.getElementById("device-list");
  if (!box) return;
  try {
    const res = await api("/devices");
    const devices = await res.json();
    if (!devices.length) { box.innerHTML = '<div class="pop-hint">No paired devices.</div>'; return; }
    box.innerHTML = devices.map(function (d) {
      return '<div class="device-row"><span>' + escapeHtml(d.name) +
        '</span><button onclick="revokeDevice(\'' + d.id + '\')">Revoke</button></div>';
    }).join("");
  } catch (e) { /* not authorized / offline — leave blank */ }
}

async function startPairing() {
  try {
    const res = await api("/pair/code", { method: "POST" });
    const data = await res.json();
    document.getElementById("pair-qr").innerHTML = data.qr_svg || "";
    document.getElementById("pair-url").textContent = data.url;
    document.getElementById("pair-code").textContent = data.code;
    document.getElementById("pair-box").hidden = false;
  } catch (e) { alert("Could not start pairing."); }
}

async function revokeDevice(id) {
  if (!confirm("Revoke this device? It will need to pair again.")) return;
  await api("/devices/" + encodeURIComponent(id), { method: "DELETE" });
  loadDevices();
}
```

> NOTE on `escapeHtml`: confirm it exists in `dashboard-utils.js` (the dashboard already renders agent fields). If it isn't there, add a minimal helper to `dashboard-utils.js`:
> ```javascript
> function escapeHtml(s) {
>   return String(s).replace(/[&<>"']/g, function (c) {
>     return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c];
>   });
> }
> ```

- [ ] **Step 3: Load devices when the Preferences popover opens**

Find where the settings popover is opened / prefs are loaded in `dashboard.js` (the `loadPrefs`/settings handler). Add a `loadDevices();` call alongside the existing prefs load so the list populates when the popover shows. If prefs load on page init, also call `loadDevices()` there.

- [ ] **Step 4: Add minimal styles for the device rows**

In `crates/kaiju-daemon/src/dashboard.rs`, in the `<style>` block, add:

```css
  .device-list { display: flex; flex-direction: column; gap: 6px; margin: 6px 0; }
  .device-row { display: flex; justify-content: space-between; align-items: center;
                gap: 8px; font-size: 13px; }
  #pair-qr svg { width: 200px; height: 200px; }
```

- [ ] **Step 5: Verify the JS unit suite still passes and the crate builds**

Run: `node --test crates/kaiju-daemon/assets/dashboard-utils.test.js && cargo build -p kaiju-daemon`
Expected: PASS. (If `escapeHtml` was added to utils, ensure the test file still passes.)

- [ ] **Step 6: Commit**

```bash
git add crates/kaiju-daemon/src/dashboard.rs crates/kaiju-daemon/assets/dashboard.js crates/kaiju-daemon/assets/dashboard-utils.js
git commit -m "feat(dashboard): pair-a-device UI with QR and revoke"
```

---

### Task 10: Docs + manual verification

**Files:**
- Modify: `README.md` (or the daemon's existing env-var docs — grep for `KAIJU_PORT` to find where env vars are documented)

- [ ] **Step 1: Document the new env vars**

Find the existing env-var documentation: `grep -rn "KAIJU_PORT" README.md docs/ 2>/dev/null`. In that same place, document:

```markdown
- `KAIJU_HOST` — bind address. Defaults to `127.0.0.1` (local only). Set `0.0.0.0`
  to listen on your LAN. When non-loopback, the dashboard's **Preferences →
  Devices** shows a QR to pair other devices; unpaired remote requests are rejected.
- `KAIJU_DEVICES` — path to the paired-devices file (default `~/.kaiju/devices.json`).
```

- [ ] **Step 2: Manual end-to-end verification**

Run the daemon exposed on the LAN and confirm the flow:

```bash
KAIJU_HOST=0.0.0.0 cargo run -p kaiju-daemon
```

Verify (record results):
1. On the host, open `http://127.0.0.1:7800/` → dashboard loads and `/agents` works (loopback trust, no token).
2. From another device on the same network, open `http://<lan-ip>:7800/` → dashboard HTML loads but data is empty/401 until paired.
3. On the host: **Preferences (⚙) → Devices → Pair a device** → a QR + code appear.
4. Scan the QR on the second device (or open the printed URL) → name it → **Pair** → it lands on the dashboard with data loading.
5. The device appears under **Devices** on the host; click **Revoke** → the device gets 401 on its next request.
6. From a device **on cellular (Wi-Fi off)**, the daemon is unreachable (confirms LAN-only / no internet exposure).

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs: KAIJU_HOST and device pairing"
```

---

## Self-Review Notes

- **Spec coverage:** `KAIJU_HOST` bind (Task 1); LAN-only exposure documented + verified (Tasks 1, 10); loopback trust anchor (Task 3); one-time code pairing (Task 5, 7); QR **in the dashboard** (Tasks 7, 9); per-device tokens reusing the existing `localStorage` bearer mechanism (Tasks 4, 7, 8); device list + revoke (Tasks 7, 9). All decisions from the brainstorm are covered; the cookie idea was deliberately simplified to the existing bearer-token path.
- **Backward compatibility:** the legacy shared `KAIJU_TOKEN` still authorizes (Task 3 predicate). Non-auth `oneshot` integration tests stay green because `is_loopback(None)` is true; the three existing **auth** tests are migrated to simulate a remote peer so they still exercise token enforcement (Task 6 Step 4).
- **Type consistency:** `Devices::tokens()`/`touch_by_token()`/`add()`/`remove()` (Task 4) are exactly what `require_auth` (Task 3) and `pair_api` (Task 7) call. `PendingCodes::issue()`/`redeem()` (Task 5) match the test and `pair_api` usage (Task 7). `kaiju_token` localStorage key is shared by the dashboard (existing), the claim page (Task 8), and is what `api()` already reads.
- **Security:** `/pair/claim` is public by necessity (unpaired devices must reach it) but only mints a token for an unguessable, single-use, 10-minute code issued by a trusted caller. `/pair/code`, `/devices`, `/devices/:id` sit behind the auth middleware (trusted-only). Tokens are never returned by `GET /devices`.
</content>
</invoke>
