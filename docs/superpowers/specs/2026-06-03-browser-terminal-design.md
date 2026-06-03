# Browser Terminal for Agents — Design

**Date:** 2026-06-03
**Status:** Approved (design), pending implementation plan

## Summary

Add a live, interactive terminal to the existing dashboard. When an operator
selects an agent in the fleet view at `http://127.0.0.1:7800/`, the detail panel
gains a **Terminal** tab that renders the agent's tmux pane in real time with
full ANSI colors and lets the operator type into it (approve prompts, send
Ctrl-C, navigate menus) — a browser-based `attach`. It is an additive tab on the
current page: no second app, no new port.

## Goals

- See an agent's live terminal output in the browser, with color, updating in
  near-real-time (~250ms).
- Type into that terminal — printable text, Enter, and control keys (Ctrl-C,
  arrows, Esc, Tab) reach the agent's tmux session.
- Stay self-contained: works offline, single binary, respects the existing
  `NEXUS_TOKEN` auth.

## Non-goals (YAGNI)

- Terminal resize protocol (the pane is a fixed size; xterm is sized to match).
- Multiple windows/panes per session (agents are single-pane).
- Session recording/playback or scrollback history beyond the initial frame.
- A dedicated full-screen terminal route — it lives in the detail panel.

## Locked decisions

| Decision | Choice |
| --- | --- |
| Interactivity | Fully interactive (type into the terminal) |
| Fidelity | "Good enough": live ANSI output + text/Enter/common control keys |
| Transport | WebSocket (axum native `ws` feature — no new backend dependency) |
| Renderer | `xterm.js`, **vendored** and served by the daemon (offline-safe) |
| Output mechanism | **A — poll `capture-pane -e` + repaint** (~250ms, change-gated) |
| Placement | Terminal **tab** inside the existing agent detail panel |

### Why poll-and-repaint (A) over pipe-pane streaming (B)

Our agents are full-screen TUIs (Claude Code, Codex, Gemini) that repaint their
own screen. Capturing the visible pane every ~250ms and repainting xterm maps
cleanly to that model and avoids FIFO/temp-file lifecycle and multi-viewer
plumbing. 250ms reads as "live". Simplest correct solution (KISS).

## Architecture

New behavior is isolated in a new module plus small, additive helpers, to
minimize edits to files a concurrent session is also touching.

### `terminal.rs` (new) — the WebSocket handler

Handler for `GET /agents/:id/terminal/ws`. On connect:

1. **Authenticate**: browsers cannot set headers on a WS handshake, so the token
   arrives as a query param (`?token=…`). Validate with the existing pure
   `auth::authorized(&configured, provided)`.
2. **Verify** the agent exists and its tmux session is live; otherwise close with
   a notice.
3. Run two halves until the socket closes or the session ends:
   - **Output task**: every `OUTPUT_POLL` (~250ms), `capture_pane_colored` the
     pane; fingerprint it; if changed, send the frame. The client repaints
     (`ESC[H` home + clear-to-end) so the screen stays stable without growing
     scrollback.
   - **Input loop**: each WS message is the raw bytes from xterm's `onData`
     (printable text *and* control sequences like `\x03`, `\x1b[A`). Hex-encode
     and `send_raw_bytes` them to tmux. This is a raw passthrough — no key
     mapping table — so Ctrl-C, arrows, Esc, Tab all work.

On disconnect or session-end, both halves stop and pipe state (none, in
approach A) is nothing to clean up.

### `tmux.rs` — three additive helpers

- `capture_pane_colored(session, lines)` — `capture-pane -e -p` (keeps ANSI).
- `send_raw_bytes(session, &[u8])` — `send-keys -t <s> -H <hex…>` (injects raw
  bytes; handles all control sequences).
- `pane_size(session)` — `display-message -p '#{pane_width}x#{pane_height}'`, so
  xterm is sized to the actual pane.

### Vendored assets + routes

- `xterm.js` and `xterm.css` committed under `crates/nexus-daemon/assets/`,
  embedded via `include_str!`, served at `GET /assets/xterm.js` and
  `/assets/xterm.css` with correct content types. The daemon remains a single
  self-contained binary with no runtime filesystem dependency.
- Route registration adds: `/agents/:id/terminal/ws`, `/assets/xterm.js`,
  `/assets/xterm.css`.
- Auth middleware: assets are public (like the dashboard HTML and `/health`);
  the WS route is exempt from the header-based middleware and authenticates
  inside the handler via the query-param token.
- `Cargo.toml`: enable axum's `ws` feature.

### `dashboard.rs` — the one shared-file UI change

The agent detail panel gains a tab switcher: **Logs** (existing polled view) and
**Terminal**. Selecting Terminal mounts an xterm instance sized to the pane and
opens a WS to `/agents/:id/terminal/ws?token=…`; `xterm.onData` → `ws.send`;
incoming frames → `term.write`. Closing the panel or switching agents closes the
socket.

## Data flow

```
browser  ──WS /agents/:id/terminal/ws?token=T──>  daemon
  auth(token) ─ ok ─ session live?
  OUT: loop 250ms: capture-pane -e → fingerprint → if changed → frame ──> xterm.write (repaint)
  IN : xterm.onData(bytes) ──> ws msg ──> hex ──> tmux send-keys -H ──> pane ──> next frame reflects it
  close / session-end → stop both halves
```

## Auth & security

- The WS respects `NEXUS_TOKEN`: no token configured ⇒ open (localhost default);
  configured ⇒ the query-param token must match.
- The interactive channel grants exactly what `POST /agents/:id/input` already
  grants — injecting keystrokes into the session. It introduces **no new
  privilege** beyond the current API; it is just a richer way to use it.
- Caveat: a query-param token can appear in server/proxy logs. Acceptable for
  the localhost-first default; documented in the README.

## Error handling

- Missing/invalid token → reject the upgrade (401 / immediate close).
- Agent not found, or session vanished mid-stream → send a "session ended"
  notice frame and close cleanly.
- `send-keys` / `capture-pane` transient failure → log and continue (a dead
  session is detected by the next capture and closes the socket).

## Testing

Per CLAUDE.md, the logic lives in pure, unit-tested functions; the IO loop stays
thin.

- **Unit**: bytes→hex encoder for `send_raw_bytes`; frame-change detector
  (reuse the monitor's `fingerprint`); token-from-query auth decision
  (`auth::authorized` already tested — add a query-param extraction test).
- **Manual / scripted**: open the Terminal tab against the `make e2e` fake agent,
  confirm live frames and that typing `1`/Enter advances it; add this to
  `VERIFICATION.md`.

## Files touched

| File | Change | Shared with concurrent session? |
| --- | --- | --- |
| `crates/nexus-daemon/src/terminal.rs` | **new** | no |
| `crates/nexus-daemon/assets/xterm.{js,css}` | **new** (vendored) | no |
| `crates/nexus-daemon/src/tmux.rs` | +3 helpers | low (additive) |
| `crates/nexus-daemon/src/api.rs` / `server.rs` | +routes, auth exemptions | **yes** |
| `crates/nexus-daemon/src/dashboard.rs` | +Terminal tab | **yes** |
| `crates/nexus-daemon/Cargo.toml` | enable axum `ws` | low |

Concurrency note: the three shared files (`api.rs`/`server.rs`, `dashboard.rs`)
are also edited by an active session. Land this on a branch and rebase, or
coordinate a quiet window, to avoid clobbering in-flight work.

## Success criteria

1. Dashboard at `:7800/` → select an agent → **Terminal** tab shows live, colored
   output updating within ~250ms of pane changes.
2. Typing text + Enter, and Ctrl-C / arrows / Esc, reach the agent and the next
   frame reflects the result.
3. Works with `NEXUS_TOKEN` set (terminal connects with the token; rejects
   without) and offline (vendored assets).
4. `make check` green; existing dashboard/log/reply/diff behavior unchanged.
