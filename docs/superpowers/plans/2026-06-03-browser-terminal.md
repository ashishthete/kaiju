# In-Browser Interactive Agent Terminal — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a live, interactive terminal (xterm.js over WebSocket) to the existing dashboard's agent detail panel, plus dashboard operator controls (create agent, per-row actions, row→terminal, copy ID).

**Architecture:** A new `terminal.rs` module hosts a WebSocket handler that polls the agent's tmux pane every ~200ms (`capture-pane -e`), pushes changed frames for xterm to repaint, and forwards browser keystrokes to tmux via `send-keys -H` (raw hex passthrough). xterm.js is vendored and served by the daemon. Three small `tmux.rs` helpers and route/auth wiring connect it. The dashboard (`dashboard.rs`) gains a Terminal tab and operator controls that reuse existing HTTP endpoints.

**Tech Stack:** Rust, axum 0.7 (native `ws` feature — no new backend dep), tokio, tmux CLI, vendored xterm.js 5.3.0 (UMD), plain JS in a single HTML page.

**Spec:** `docs/superpowers/specs/2026-06-03-browser-terminal-design.md`

**Concurrency note:** A second session edits `dashboard.rs`, `api.rs`, `auth.rs`. Do all work on a branch and rebase before merge.

---

### Task 1: Branch + scaffolding (axum ws feature, module, vendored assets)

**Files:**
- Modify: `crates/nexus-daemon/Cargo.toml`
- Modify: `crates/nexus-daemon/src/lib.rs`
- Create: `crates/nexus-daemon/src/terminal.rs`
- Create: `crates/nexus-daemon/assets/xterm.js`
- Create: `crates/nexus-daemon/assets/xterm.css`

- [ ] **Step 1: Create a feature branch**

```bash
cd /Users/ashishthete/work/personal/AgentNexus
git checkout -b feat/browser-terminal
```

- [ ] **Step 2: Enable axum's `ws` feature**

Edit `crates/nexus-daemon/Cargo.toml`, change the axum dependency line:

```toml
axum = { workspace = true, features = ["ws"] }
```

- [ ] **Step 3: Vendor xterm.js + xterm.css (offline-safe)**

```bash
mkdir -p crates/nexus-daemon/assets
curl -fL https://cdn.jsdelivr.net/npm/xterm@5.3.0/lib/xterm.js -o crates/nexus-daemon/assets/xterm.js
curl -fL https://cdn.jsdelivr.net/npm/xterm@5.3.0/css/xterm.css -o crates/nexus-daemon/assets/xterm.css
# sanity: the UMD bundle defines a global Terminal
grep -c "Terminal" crates/nexus-daemon/assets/xterm.js
wc -c crates/nexus-daemon/assets/xterm.js crates/nexus-daemon/assets/xterm.css
```
Expected: `xterm.js` ~280KB, `xterm.css` present.

- [ ] **Step 4: Declare the module**

Edit `crates/nexus-daemon/src/lib.rs` — add this line alphabetically among the `pub mod` block (after `pub mod tmux;`):

```rust
pub mod terminal;
```

- [ ] **Step 5: Create a placeholder `terminal.rs` so the workspace compiles**

Create `crates/nexus-daemon/src/terminal.rs`:

```rust
//! In-browser interactive terminal: a WebSocket that mirrors an agent's tmux
//! pane (poll + repaint) and forwards keystrokes (raw `send-keys -H`).
//! Also serves the vendored xterm.js assets.
```

- [ ] **Step 6: Verify it builds**

Run: `cargo build -p nexus-daemon`
Expected: compiles (one harmless `dead_code`/unused warning is fine at this stage).

- [ ] **Step 7: Commit**

```bash
git add crates/nexus-daemon/Cargo.toml crates/nexus-daemon/src/lib.rs crates/nexus-daemon/src/terminal.rs crates/nexus-daemon/assets/
git commit -m "feat(terminal): scaffold module, enable axum ws, vendor xterm.js"
```

---

### Task 2: tmux helpers (`capture_pane_colored`, `send_raw_bytes`, `pane_size`)

**Files:**
- Modify: `crates/nexus-daemon/src/tmux.rs`
- Test: `crates/nexus-daemon/src/tmux.rs` (new `#[cfg(test)] mod tests`)

- [ ] **Step 1: Write failing tests for the two pure helpers**

Append to `crates/nexus-daemon/src/tmux.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_bytes_encodes_each_byte_as_two_digits() {
        // Ctrl-C, then ESC [ A (up arrow)
        assert_eq!(hex_bytes(&[0x03, 0x1b, 0x5b, 0x41]), vec!["03", "1b", "5b", "41"]);
        assert_eq!(hex_bytes(&[0x00, 0xff]), vec!["00", "ff"]);
        assert!(hex_bytes(&[]).is_empty());
    }

    #[test]
    fn parse_size_reads_width_x_height() {
        assert_eq!(parse_size("80x24\n"), Some((80, 24)));
        assert_eq!(parse_size("  200x50  "), Some((200, 50)));
        assert_eq!(parse_size("nope"), None);
        assert_eq!(parse_size("80xfoo"), None);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail to compile**

Run: `cargo test -p nexus-daemon --lib tmux::tests`
Expected: FAIL — `cannot find function hex_bytes` / `parse_size`.

- [ ] **Step 3: Implement the helpers**

In `crates/nexus-daemon/src/tmux.rs`, add these free functions just **above** `impl TmuxManager {`:

```rust
/// Encode bytes as tmux `send-keys -H` hex arguments (one per byte). Pure.
fn hex_bytes(bytes: &[u8]) -> Vec<String> {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Parse tmux `#{pane_width}x#{pane_height}` output (e.g. "80x24"). Pure.
fn parse_size(s: &str) -> Option<(u16, u16)> {
    let (w, h) = s.trim().split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?))
}
```

Then add these three methods inside `impl TmuxManager` (after `send_interrupt`):

```rust
    /// Capture the *visible* pane (current screen) with ANSI escapes preserved
    /// (`-e`), for rendering in a browser terminal. Unlike `capture_pane`, it
    /// omits `-S` so the result is exactly one screen — ideal for repaint.
    pub fn capture_pane_colored(session_name: &str) -> Result<String> {
        let output = Command::new("tmux")
            .args(["capture-pane", "-t", session_name, "-e", "-p"])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(NexusError::Tmux(format!(
                "failed to capture pane '{session_name}': {stderr}"
            )));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Inject raw bytes into the session as if typed, using `send-keys -H`
    /// (hex). This passes through control sequences (Ctrl-C, arrows, Esc, …)
    /// without any key-name mapping.
    pub fn send_raw_bytes(session_name: &str, bytes: &[u8]) -> Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let mut args = vec![
            "send-keys".to_string(),
            "-t".to_string(),
            session_name.to_string(),
            "-H".to_string(),
        ];
        args.extend(hex_bytes(bytes));

        let output = Command::new("tmux").args(&args).output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(NexusError::Tmux(format!(
                "failed to send raw bytes to '{session_name}': {stderr}"
            )));
        }
        Ok(())
    }

    /// Report the pane size as `(cols, rows)` so the browser terminal can match
    /// it (avoids line-wrap mismatch).
    pub fn pane_size(session_name: &str) -> Result<(u16, u16)> {
        let output = Command::new("tmux")
            .args([
                "display-message",
                "-p",
                "-t",
                session_name,
                "#{pane_width}x#{pane_height}",
            ])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(NexusError::Tmux(format!(
                "failed to read pane size '{session_name}': {stderr}"
            )));
        }

        let raw = String::from_utf8_lossy(&output.stdout);
        parse_size(&raw).ok_or_else(|| NexusError::Tmux(format!("unparseable pane size: {raw:?}")))
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p nexus-daemon --lib tmux::tests`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/nexus-daemon/src/tmux.rs
git commit -m "feat(tmux): capture_pane_colored, send_raw_bytes, pane_size helpers"
```

---

### Task 3: Terminal module — asset, size, and WebSocket handlers

**Files:**
- Modify: `crates/nexus-daemon/src/terminal.rs`

- [ ] **Step 1: Implement the full module**

Replace the contents of `crates/nexus-daemon/src/terminal.rs` with:

```rust
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
struct Size {
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
    let s = session.to_string();
    let _ = tokio::task::spawn_blocking(move || TmuxManager::send_raw_bytes(&s, &bytes)).await;
}
```

- [ ] **Step 2: Verify it builds**

Run: `cargo build -p nexus-daemon`
Expected: compiles. (Handlers are unused until Task 4 wires routes — `dead_code` warnings are expected and will clear in Task 4.)

- [ ] **Step 3: Commit**

```bash
git add crates/nexus-daemon/src/terminal.rs
git commit -m "feat(terminal): asset, size, and websocket handlers"
```

---

### Task 4: Wire routes + auth exemptions

**Files:**
- Modify: `crates/nexus-daemon/src/api.rs:15-30` (the `routes()` function)
- Modify: `crates/nexus-daemon/src/auth.rs` (the `require_auth` exemption check)
- Test: `crates/nexus-daemon/tests/api.rs`

- [ ] **Step 1: Register the four routes**

In `crates/nexus-daemon/src/api.rs`, inside `pub fn routes()`, add these lines immediately after the `.route("/agents/:id/input", post(send_input))` line:

```rust
        .route(
            "/agents/:id/terminal/ws",
            get(crate::terminal::terminal_ws),
        )
        .route(
            "/agents/:id/terminal/size",
            get(crate::terminal::terminal_size),
        )
        .route("/assets/xterm.js", get(crate::terminal::xterm_js))
        .route("/assets/xterm.css", get(crate::terminal::xterm_css))
```

- [ ] **Step 2: Exempt public/terminal paths from header auth**

In `crates/nexus-daemon/src/auth.rs`, find the early-return in `require_auth`:

```rust
    if path == "/health" || path == "/" {
        return next.run(req).await;
    }
```

Replace it with:

```rust
    // Public: liveness, the dashboard page, and the vendored assets. The
    // terminal WebSocket authenticates itself from its query string (browsers
    // can't set headers on a WS handshake), so it is exempt here too.
    if path == "/health"
        || path == "/"
        || path.starts_with("/assets/")
        || path.ends_with("/terminal/ws")
    {
        return next.run(req).await;
    }
```

- [ ] **Step 3: Write a failing test that assets are public**

Append to `crates/nexus-daemon/tests/api.rs` (follow the existing `get_request`/`oneshot` pattern already in that file):

```rust
#[tokio::test]
async fn xterm_asset_is_served_publicly() {
    let app = nexus_daemon::server::build_app(nexus_daemon::server::AppState::new());
    let res = app
        .oneshot(get_request("/assets/xterm.js"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let ct = res.headers().get("content-type").unwrap().to_str().unwrap();
    assert!(ct.contains("javascript"), "got content-type {ct}");
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p nexus-daemon --test api xterm_asset_is_served_publicly`
Expected: PASS. (If `StatusCode` or `get_request` is unresolved, add `use axum::http::StatusCode;` — match the imports already at the top of `tests/api.rs`.)

- [ ] **Step 5: Full build to confirm no dead_code warnings remain**

Run: `cargo clippy -p nexus-daemon --all-targets -- -D warnings`
Expected: clean (the Task 3 handlers are now used).

- [ ] **Step 6: Commit**

```bash
git add crates/nexus-daemon/src/api.rs crates/nexus-daemon/src/auth.rs crates/nexus-daemon/tests/api.rs
git commit -m "feat(terminal): wire routes and auth exemptions"
```

---

### Task 5: Dashboard Terminal tab (xterm wiring)

**Files:**
- Modify: `crates/nexus-daemon/src/dashboard.rs` (the `PAGE` HTML/JS string)

This task is browser UI; verification is manual (Task 7). Make these four edits to the `PAGE` string.

- [ ] **Step 1: Load xterm assets + add tab/terminal styles**

In the `<head>`, immediately after the `<title>AgentNexus</title>` line, add:

```html
<link rel="stylesheet" href="/assets/xterm.css">
<script src="/assets/xterm.js"></script>
```

In the `<style>` block, add these rules (before the closing `</style>`):

```css
  .tabs { display: flex; gap: .25rem; margin-bottom: .5rem; }
  .tab { font-size: .8rem; padding: .25rem .7rem; }
  .tab.active { background: #3b82f633; border-color: #3b82f6aa; }
  #d-term { width: 100%; height: 24rem; }
  #d-term[hidden] { display: none; }
```

- [ ] **Step 2: Add the tab switcher + terminal container to the detail panel**

In the detail panel, replace this existing line:

```html
    <pre class="logs" id="d-logs">Loading…</pre>
```

with:

```html
    <div class="tabs">
      <button id="tab-logs" class="tab" onclick="showTab('logs')">Logs</button>
      <button id="tab-term" class="tab active" onclick="showTab('term')">Terminal</button>
    </div>
    <pre class="logs" id="d-logs" hidden>Loading…</pre>
    <div id="d-term"></div>
```

- [ ] **Step 3: Add terminal JS state + functions**

In the `<script>`, immediately after the line `let token = localStorage.getItem("nexus_token") || "";`, add:

```js
let term = null, ws = null, activeTab = "term";

function showTab(which) {
  activeTab = which;
  const onTerm = which === "term";
  document.getElementById("tab-logs").classList.toggle("active", !onTerm);
  document.getElementById("tab-term").classList.toggle("active", onTerm);
  document.getElementById("d-logs").hidden = onTerm;
  document.getElementById("d-term").hidden = !onTerm;
  if (onTerm) openTerminal(); else { closeTerminal(); refreshDetail(); }
}

async function openTerminal() {
  closeTerminal();
  if (!selected || !window.Terminal) return;
  let cols = 80, rows = 24;
  try {
    const res = await api("/agents/" + selected + "/terminal/size");
    if (res.ok) { const s = await res.json(); cols = s.cols || 80; rows = s.rows || 24; }
  } catch (e) { /* use defaults */ }
  term = new Terminal({ cols, rows, fontSize: 13, cursorBlink: true,
                        convertEol: false, scrollback: 0 });
  term.open(document.getElementById("d-term"));
  const proto = location.protocol === "https:" ? "wss" : "ws";
  const q = token ? ("?token=" + encodeURIComponent(token)) : "";
  ws = new WebSocket(proto + "://" + location.host +
                     "/agents/" + selected + "/terminal/ws" + q);
  ws.onmessage = (e) => { if (term) term.write(e.data); };
  ws.onclose = () => { if (term) term.write("\r\n[disconnected]\r\n"); };
  term.onData((d) => { if (ws && ws.readyState === 1) ws.send(d); });
}

function closeTerminal() {
  if (ws) { try { ws.close(); } catch (e) {} ws = null; }
  if (term) { try { term.dispose(); } catch (e) {} term = null; }
  const el = document.getElementById("d-term");
  if (el) el.innerHTML = "";
}
```

- [ ] **Step 4: Hook tab lifecycle into select/close, and guard the log poll**

Replace the existing `select(id)` and `closeDetail()` functions:

```js
function select(id) {
  selected = id;
  document.getElementById("detail").hidden = false;
  document.getElementById("d-id").textContent = id.slice(0, 10);
  document.getElementById("d-logs").textContent = "Loading…";
  note("");
  showTab("term");   // open on the live terminal by default
}
function closeDetail() {
  closeTerminal();
  selected = null;
  document.getElementById("detail").hidden = true;
}
```

In `refreshDetail()`, add this guard as the **first line of the function body** (so the 2s poll doesn't fetch logs while the Terminal tab is showing):

```js
  if (activeTab === "term") return;
```

- [ ] **Step 5: Build and smoke-compile the page string**

Run: `cargo build -p nexus-daemon`
Expected: compiles (the page is a static string; this just confirms no Rust-level breakage).

- [ ] **Step 6: Commit**

```bash
git add crates/nexus-daemon/src/dashboard.rs
git commit -m "feat(dashboard): live terminal tab in the agent detail panel"
```

---

### Task 6: Dashboard operator controls (create, per-row actions, copy ID)

**Files:**
- Modify: `crates/nexus-daemon/src/dashboard.rs` (the `PAGE` HTML/JS string)

All four enhancements use existing endpoints. Browser UI — verified manually in Task 7.

- [ ] **Step 1: Add the "New agent" button + form markup**

Immediately after the `<div class="sub">…</div>` line (above `<div class="counts">`), add:

```html
  <div style="margin-bottom:1rem">
    <button onclick="toggleNew()">+ New agent</button>
    <form id="newform" hidden onsubmit="createAgent(event)"
          style="margin-top:.5rem; display:flex; gap:.5rem; flex-wrap:wrap; align-items:center">
      <select id="n-type">
        <option value="claude">claude</option>
        <option value="codex">codex</option>
        <option value="gemini">gemini</option>
      </select>
      <input id="n-ws" placeholder="workspace path" value="." style="flex:1; min-width:14rem">
      <input id="n-model" placeholder="model (optional)">
      <input id="n-prompt" placeholder="prompt" style="flex:2; min-width:18rem">
      <label style="font-size:.85rem"><input type="checkbox" id="n-isolate"> isolate</label>
      <button type="submit">Start</button>
    </form>
  </div>
```

- [ ] **Step 2: Add `<style>` for per-row action buttons**

In the `<style>` block add:

```css
  td.actions { white-space: nowrap; }
  td.actions button { padding: .15rem .45rem; font-size: .75rem; margin-left: .25rem; }
```

- [ ] **Step 3: Add an Actions column header + cells**

In the table header row, add an `<th>` after `<th>Task</th>`:

```html
      <th>Task</th><th>Actions</th>
```

In `render(agents)`, in the row template returned by `.map(...)`, add this cell right after the `<td class="prompt">…</td>` line (before the closing `</tr>`):

```html
      <td class="actions" onclick="event.stopPropagation()">
        <button title="Copy full ID" onclick="copyId('${a.id}')">⧉</button>
        <button title="Interrupt" onclick="rowAct('${a.id}','interrupt')">⎋</button>
        <button title="Stop" onclick="rowAct('${a.id}','stop')">■</button>
        <button title="Remove" onclick="removeAgent('${a.id}')">✕</button>
      </td>
```

Also add a `title` with the full ID to the existing ID cell — replace:

```html
      <td class="id">${a.id.slice(0,10)}</td>
```

with:

```html
      <td class="id" title="${a.id}">${a.id.slice(0,10)}</td>
```

- [ ] **Step 4: Add the create/action/copy JS functions**

In the `<script>`, add these functions (e.g. just before `async function refresh()`):

```js
function toggleNew() {
  const f = document.getElementById("newform");
  f.hidden = !f.hidden;
}

async function createAgent(ev) {
  ev.preventDefault();
  const body = {
    agent_type: document.getElementById("n-type").value,
    workspace: document.getElementById("n-ws").value,
    prompt: document.getElementById("n-prompt").value || null,
    isolate: document.getElementById("n-isolate").checked,
    auto_start: true,
  };
  const model = document.getElementById("n-model").value;
  if (model) body.model = model;
  try {
    const res = await api("/agents", {
      method: "POST", headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    });
    if (res.ok) { document.getElementById("newform").hidden = true; refresh(); }
    else { alert((await res.json()).error || "create failed"); }
  } catch (e) { alert("create failed"); }
}

async function rowAct(id, path) {
  try { await api("/agents/" + id + "/" + path, { method: "POST" }); refresh(); }
  catch (e) { /* ignore */ }
}

async function removeAgent(id) {
  if (!confirm("Remove agent " + id.slice(0, 10) + "? (stops it if running)")) return;
  try {
    await api("/agents/" + id, { method: "DELETE" });
    if (id === selected) closeDetail();
    refresh();
  } catch (e) { /* ignore */ }
}

function copyId(id) {
  navigator.clipboard.writeText(id).then(
    () => { /* copied */ },
    () => { window.prompt("Copy agent ID:", id); }
  );
}
```

- [ ] **Step 5: Add full ID + copy button to the detail header**

In the detail panel head, replace:

```html
      <span class="id" id="d-id"></span>
```

with:

```html
      <span class="id" id="d-id"></span>
      <button title="Copy full ID" onclick="copyId(selected)">⧉ copy id</button>
```

- [ ] **Step 6: Build**

Run: `cargo build -p nexus-daemon`
Expected: compiles.

- [ ] **Step 7: Commit**

```bash
git add crates/nexus-daemon/src/dashboard.rs
git commit -m "feat(dashboard): create form, per-row actions, copy full ID"
```

---

### Task 7: Verification, docs, and the full gate

**Files:**
- Modify: `README.md`
- Modify: `VERIFICATION.md`

- [ ] **Step 1: Run the full pre-commit gate**

Run: `make check`
Expected: `cargo fmt --check` clean, `clippy -D warnings` clean, all tests pass (the existing suite plus the 2 tmux tests and 1 asset test added here).

- [ ] **Step 2: Manual end-to-end check against a real agent**

```bash
# fresh daemon on a test port
NEXUS_PORT=7850 NEXUS_STATE=/tmp/nx-term/state.json cargo run -p nexus-daemon &
sleep 2
WS=$(mktemp -d); git -C "$WS" init -q && git -C "$WS" commit --allow-empty -qm init
agentnexus --url http://127.0.0.1:7850 start -t claude -w "$WS" -p "say hi"
```
Then open `http://127.0.0.1:7850/` in a browser and confirm:
- The agent row shows; the **Actions** column has copy/interrupt/stop/remove.
- Clicking the row opens the detail panel on the **Terminal** tab.
- The terminal shows live, colored output and updates within ~250ms.
- Typing `1` + Enter (to answer a prompt) and Ctrl-C reach the agent.
- The **Logs** tab still shows the polled text view.
- "+ New agent" creates and auto-starts an agent that appears on refresh.
- The ⧉ copy button copies the full ID.

Tear down: `kill %1; rm -rf "$WS" /tmp/nx-term`.

- [ ] **Step 3: Add the README note**

In `README.md`, under the dashboard/Running section, add:

```markdown
The dashboard (`/`) includes a live **Terminal** tab per agent (xterm.js over a
WebSocket): real-time colored output and interactive input (type, Enter, Ctrl-C,
arrows). You can also create agents, run per-row actions (interrupt/stop/remove),
and copy IDs from the page. The terminal respects `NEXUS_TOKEN` (passed as a
`?token=` query param on the WebSocket).
```

- [ ] **Step 4: Add a verification entry**

Append the Task-7 Step-2 checklist to `VERIFICATION.md` under a new "Browser terminal" heading.

- [ ] **Step 5: Commit**

```bash
git add README.md VERIFICATION.md
git commit -m "docs: document the in-browser agent terminal"
```

- [ ] **Step 6: (Optional) open a PR**

```bash
git push -u origin feat/browser-terminal
gh pr create --base main --title "In-browser interactive agent terminal" \
  --body "Live xterm.js terminal in the dashboard detail panel + dashboard operator controls. See docs/superpowers/specs/2026-06-03-browser-terminal-design.md."
```

---

## Notes for the implementer

- **No new backend dependency.** axum's `ws` feature is the only addition; tokio already has `full` features.
- **Why poll+repaint:** agents are full-screen TUIs that repaint themselves; capturing the visible pane and repainting xterm is simpler and robust vs. streaming. ~200ms reads as live.
- **Raw input passthrough:** `send-keys -H <hex>` injects bytes verbatim, so control sequences work with no mapping table.
- **Security:** the WS grants exactly what `POST /agents/:id/input` already grants. Query-param token can appear in logs — acceptable for the localhost default; noted in the README.
