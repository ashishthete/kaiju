# Kaiju Pet — Native Shell Rebuild

**Date:** 2026-07-17
**Status:** Decision recorded; ready to execute in a fresh session
**Supersedes:** the window/shell layer of the eframe/egui pet (see
`2026-07-16-kaiju-pet-design.md`). The pet's *logic* is kept; only the GUI shell
is replaced.

## Why

The egui/eframe pet works over normal windows but **cannot float over another
app's full-screen Space**, and cannot take keyboard focus without activating.
Proven exhausted (all committed on `main`, all ineffective for full-screen):
`NSWindowCollectionBehavior::CanJoinAllSpaces | FullScreenAuxiliary | Stationary`,
`setLevel(NSScreenSaverWindowLevel)` (1000), and
`NSApplicationActivationPolicy::Accessory`. Root cause: winit owns the `NSWindow`
and resets these; egui also stops rendering when occluded, so a per-frame
re-apply can't win. A true desktop pet must be a **non-activating `NSPanel`** from
creation — which winit does not provide. This was flagged as a known risk in the
original spec.

## Chosen approach: Tauri + tauri-nspanel

Rebuild only the shell; reuse the tested Rust core.

- **Reuse unchanged** (already isolated + unit-tested in `crates/kaiju-pet`):
  `model` (status wire contract), `client` (agents/input/logs + error mapping),
  `reducer` (aggregate/order/transition/unread), `validate`, `ansi`, `prefs`,
  and the `poller` loop/backoff. Extract these into a plain library crate
  (`kaiju-pet-core`) with **no egui dependency**.
- **Replace** the egui window + widgets with a **Tauri** app:
  - Rust backend hosts `kaiju-pet-core`; exposes Tauri commands
    (`get_snapshot`, `send_reply`, `fetch_logs`) and emits events to the webview
    on each poll.
  - Webview frontend renders the pet in HTML/CSS/JS — trivial rich animation
    (CSS keyframes: eye blink, limb wiggle, breathing, per-state banners) and
    easy real art (SVG/PNG/sprite-sheet). This directly answers the "boring, no
    eye/feather movement" and "status banner + actions" asks.
  - **`tauri-nspanel`** plugin converts the window into a non-activating
    `NSPanel` with `canJoinAllSpaces | fullScreenAuxiliary` → floats over
    full-screen Spaces AND takes text focus without stealing app focus. This is
    the specific capability egui could not provide.

Why Tauri over pure Swift: keeps the whole daemon-facing core in Rust (already
written and tested); only the presentation moves to web tech. A Swift/AppKit
rebuild would force reimplementing the core or a C-FFI bridge.

## Feature parity checklist (from this session's feedback)

- [ ] Floats over all Spaces AND full-screen apps (the blocker) — `tauri-nspanel`.
- [ ] Take reply focus without activating the app (non-activating panel).
- [ ] Draggable anywhere; position persisted (reuse `prefs`).
- [ ] Rich constant animation: blink, limb/feather motion, breathing; per-state.
- [ ] Auto status banner on stuck/attention (task title + state + spinner), no
      click required — matches the reference card.
- [ ] State-specific actions in the banner (Reply / Open dashboard / Retry).
- [ ] Mascot: **kaiju** monster (placeholder art at
      `crates/kaiju-pet/assets/pet.png`; commission real art).
- [ ] Right-click / menu Quit; reduced-motion honored.

## Boundaries kept from the original spec

Daemon unchanged (`GET /agents` 2s poll, `POST /agents/:id/input`,
`GET /agents/:id/logs`); loopback default `http://127.0.0.1:7800`; reply ≤4 KiB,
no control chars, no auto-retry; terminal statuses disable reply; reply text
never logged, cleared on switch. Status wire strings are the **no-underscore**
Debug-lowercased form (`waitingforinput`) — the `model::parse_status` contract
test must carry over.

## First execution steps (fresh session)

1. `superpowers:brainstorming` to confirm Tauri vs alternatives, then
   `superpowers:writing-plans` for the task breakdown.
2. Extract `crates/kaiju-pet-core` (move the pure modules; drop egui).
3. Scaffold the Tauri app; wire `tauri-nspanel`; verify a bare panel floats over
   a full-screen app **before** building any UI (de-risk the core assumption
   first).
4. Port the UI (collapsed pet + bubble + banner) to the webview; then animation
   and art.

## Note on the current egui pet

It remains on `main` and works over normal windows. Decide whether to keep it as
a fallback or remove it once the Tauri shell reaches parity. The accessory
activation policy (no Dock icon) it currently sets can be reverted if the egui
build is kept around.
