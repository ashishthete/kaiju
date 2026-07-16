# Kaiju Pet Desktop Companion — Design

**Date:** 2026-07-16

**Status:** Draft specification

**Scope:** macOS-first local desktop companion; portable architecture for later
Linux and Windows support

## Summary

Add an optional **Kaiju Pet**: a small, draggable, always-on-top desktop
companion that reflects the state of Kaiju agents, calls attention to agents
that need the operator, and lets the operator send a short reply without opening
the full dashboard.

The pet is a companion to the existing daemon, not another agent runtime. It
reads agent state from the daemon and sends replies through the existing HTTP
API. The full dashboard remains the place for terminal access, diffs, agent
creation, judging, promotion, settings, and destructive actions.

This specification interprets “pet” as a native floating desktop character,
similar to a lightweight desktop pet, rather than a widget embedded only in the
Kaiju dashboard.

## Problem

Kaiju can already detect when an agent is waiting, stuck, limited, or in error.
It can also raise browser/native notifications and accept input in the dashboard.
Those capabilities are separate:

- A notification tells the operator that something happened, but does not offer
  a convenient reply surface.
- Replying requires finding and opening the dashboard or using the CLI.
- The dashboard is intentionally information-dense and is more than the
  operator needs for a quick “yes”, clarification, or follow-up prompt.

The operator needs a persistent but unobtrusive surface that shows fleet health
at a glance and turns an attention event into a one-step reply.

## Goals

- Show a small Kaiju character on the desktop while the companion is running.
- Represent aggregate fleet state through a small set of clear visual states.
- Surface new attention events in a speech bubble within two daemon poll cycles.
- Let the operator select an agent and send one line of text to its live session.
- Stay useful when the main dashboard is closed.
- Reuse existing daemon APIs and authentication behavior in the first version.
- Remain optional: Kaiju daemon and CLI behavior must not depend on the pet.
- Keep all agent execution, state detection, and input injection in the daemon.

## Non-goals

- Replacing the dashboard or its full interactive terminal.
- Creating, stopping, removing, judging, or promoting agents from the pet.
- Sending raw terminal control keys, files, images, or multiline pasted content.
- Acting as a general chatbot when no Kaiju agent is selected.
- Reading arbitrary desktop notifications from other applications.
- Voice input, speech synthesis, global keyboard shortcuts, or autonomous replies.
- A plugin/theme marketplace or user-authored pet behaviors in the first version.
- Perfect always-on-top and transparent-window behavior on every Linux window
  manager in the first release.

## Product principles

1. **Glanceable:** the pet conveys “all good” or “needs you” without requiring
   text to be read.
2. **Quiet by default:** animation and bubbles communicate state without
   repeatedly stealing focus or playing sound.
3. **Human-controlled:** the pet never sends agent input by itself.
4. **Safe scope:** quick replies are supported; consequential fleet management
   stays in the dashboard.
5. **Local-first:** the pet talks to the local Kaiju daemon and does not require
   a cloud service.

## User experience

### Launch and placement

- Launch with `kaiju pet` or the standalone `kaiju-pet` executable.
- Default daemon URL: `http://127.0.0.1:7800`.
- The pet opens near the bottom-right of the primary display, inside the usable
  screen area.
- The window is frameless, transparent, always on top, and does not take focus
  until clicked.
- Dragging the pet moves it. The last position is restored on the next launch
  and clamped to a currently connected display.
- A right-click/context menu provides **Open dashboard**, **Pause alerts**,
  **Preferences**, and **Quit pet**.
- Closing or quitting the pet does not stop the daemon or any agents.

### Collapsed state

The normal view is only the character plus a small optional badge:

- No badge: no agent needs attention.
- Number badge: count of agents in an attention state.
- Connection badge: daemon cannot be reached.

Clicking the character opens the interaction bubble. Clicking outside the
bubble collapses it without changing agent state or dismissing unread events.

### Interaction bubble

The expanded bubble contains:

1. Current status summary, such as “3 working · 1 needs you”.
2. An agent selector when more than one agent needs attention.
3. Selected agent identity: short ID, CLI type, workspace name, and status.
4. The agent's original prompt, truncated to two lines.
5. Up to five recent plain-text log lines, fetched only when the bubble opens.
6. A single-line reply field and **Send** button for a live agent.
7. **Open dashboard** for full context and controls.

Pressing Enter sends the reply. Shift+Enter does nothing because the field is
single-line. The input is trimmed; an empty value is not sent. After a successful
send, the field clears and the bubble shows “Sent”. The bubble stays open so the
operator can observe the next status poll.

The input is disabled for terminal states (`completed`, `stopped`, and `error`)
and while disconnected. In those cases the primary action is **Open dashboard**.

### Attention selection

When several agents need attention, selection order is:

1. `waitingforinput`
2. `stuck`
3. `limited`
4. `error`
5. Most recently updated agent within the same status

The operator can move between attention agents without dismissing them. An
event becomes read when its agent is shown in the expanded bubble. Read state
affects only the pet badge; it does not modify daemon state.

### Visual states

| Fleet condition | Pet behavior | Bubble accent |
| --- | --- | --- |
| Daemon unreachable | Sleeps/fades with a broken-link badge | Gray |
| No active agents | Calm idle animation | Neutral |
| One or more agents running/starting | Working animation | Blue |
| Waiting for input | Raises head and shows one speech bubble | Amber |
| Stuck | Concerned pose with a slow pulse | Amber |
| Limited | Resting pose with an hourglass | Purple |
| Error | Alert pose, no continuous shaking | Red |
| Agent newly completed | One short celebration, then aggregate state | Green |

Animation must stop when the OS reduced-motion preference is active. State must
also be identifiable by icon and text; color alone is insufficient.

## Notification behavior

The pet creates an event only when an agent transitions into a configured state.
It does not alert again on every poll while the same state persists.

Default notification policy:

| Transition | Bubble | Optional sound | Unread badge |
| --- | --- | --- | --- |
| Waiting for input | Yes | On | Yes |
| Stuck | Yes | On | Yes |
| Limited | Yes | Off | Yes |
| Error | Yes | On | Yes |
| Completed | Brief celebration | Off | No |
| Starting/running/stopped | No | Off | No |

- Sound is one short, pet-specific cue and can be muted globally.
- **Pause alerts** suppresses bubbles, animation escalation, and sound for a
  chosen duration while polling continues. Unread events still accumulate.
- On startup, the pet builds its initial state without replaying historical
  transition sounds. Existing attention agents appear in the badge and bubble.
- After reconnecting, the first successful snapshot follows the same startup
  rule, preventing an alert storm.
- The pet does not duplicate native OS notifications in v1. Operators may use
  either existing `KAIJU_DESKTOP_NOTIFY` notifications, the pet, or both.

An event is locally identified by `(agent_id, status, updated_at)`. The pet keeps
a bounded in-memory queue of the latest 100 events and persists only unread
identifiers needed across a pet restart.

## Chosen architecture

### Separate optional native binary

Add a `kaiju-pet` workspace crate that produces a standalone `kaiju-pet`
executable. The existing `kaiju` CLI may later expose `kaiju pet` as a thin
launcher, but the GUI process remains separate from the daemon.

The GUI should use Rust `eframe`/`egui` for a small transparent native window.
This keeps the companion in the existing language and avoids embedding a browser
or adding a JavaScript build pipeline. The dependency is isolated to the
optional pet crate so daemon and CLI builds do not take a GUI dependency.

macOS is the release target for v1. The application logic, HTTP client, and
state reducer must remain platform-neutral; platform-specific window behavior
is kept behind a small window adapter for later Linux and Windows support.

### Reuse existing daemon APIs

The MVP adds no daemon endpoint and no push transport.

| Purpose | Existing API |
| --- | --- |
| Fleet snapshot | `GET /agents` |
| Reply to an agent | `POST /agents/:id/input` with `{ "text": "..." }` |
| Recent context | `GET /agents/:id/logs` |
| Full management | Open the existing dashboard URL |

The pet polls `GET /agents` every two seconds, matching the daemon monitor's
two-second interval. A faster pet poll would not provide materially fresher
status and would add unnecessary work. Recent logs are loaded on demand, not on
every fleet poll.

An SSE or WebSocket event feed is deferred. It should be considered only if
measurement shows that snapshot polling is too expensive or too slow.

### Internal components

The pet follows a small single-responsibility split:

- `client`: typed HTTP calls and response validation.
- `poller`: timed fleet snapshots, reconnection, and bounded backoff.
- `reducer`: pure previous-snapshot/current-snapshot transition detection,
  aggregate status, ordering, unread state, and visual-state selection.
- `ui`: renders the pet, bubble, selector, input, and preferences.
- `window`: platform-specific position, focus, transparency, and always-on-top
  behavior.
- `preferences`: local persistence for position and pet-only settings.

The UI thread never performs network IO. A background worker owns the HTTP
client and communicates with the UI through bounded channels. Only the newest
fleet snapshot needs to be retained; stale snapshots may be dropped.

## Data flow

```text
Kaiju daemon
  GET /agents every 2s
       |
       v
pet poller -> pure state reducer -> aggregate visual state -> pet UI
                         |                   |
                         |                   +-> bubble / badge / animation
                         +-> transition queue -> optional sound

operator types a reply
  -> POST /agents/:id/input
  -> daemon validates live agent
  -> tmux send_keys(text + Enter)
  -> next fleet poll reflects the resulting state
```

## Local preferences

Pet-only preferences are stored separately from daemon settings because they
describe a local GUI, not agent defaults.

```json
{
  "daemon_url": "http://127.0.0.1:7800",
  "window": { "display": "primary", "x": 1200, "y": 760 },
  "sound": true,
  "reduced_motion": false,
  "pause_until": null,
  "notify": {
    "waitingforinput": true,
    "stuck": true,
    "limited": true,
    "error": true,
    "completed": true
  }
}
```

The default path is under the platform's normal application-config directory.
No preference is written into the user's repository.

`reduced_motion` defaults to the OS preference when available; an explicit user
choice overrides it.

## Authentication and security

- The default connection is loopback, which the daemon already trusts.
- If `daemon_url` is non-loopback, the pet requires an explicit bearer token and
  follows the daemon's existing authentication rules.
- A configured token must be stored in the OS credential store, not in the
  plaintext preferences file. Non-loopback token setup may be deferred from the
  first macOS release; the UI must clearly reject unsupported remote use.
- The pet sends only line input through the auth-gated HTTP endpoint. It does
  not connect to the raw terminal WebSocket.
- Reply text is never logged by the pet and is cleared from memory/UI after a
  successful send or when switching agents.
- The pet must not render terminal output as rich text or HTML. Log context is
  plain text with ANSI/control sequences removed and a fixed display limit.
- Workspace paths are shown as the final directory name by default. Full paths
  appear only in an explicit details view.
- No telemetry or external network request is introduced.

## Input constraints

- Maximum reply size: 4 KiB UTF-8 in the pet, even if the daemon accepts more.
- Newlines and NUL/control characters are rejected before sending.
- The agent ID comes only from the latest daemon snapshot, never free-form user
  input.
- The Send button is disabled while a send is in flight, preventing duplicate
  submissions.
- A failed send keeps the text in the field and shows a retryable error.
- The pet never retries a reply automatically because duplicate agent input can
  be consequential.

## Failure handling

- **Daemon unavailable:** enter disconnected state; retry after 2s, then use a
  capped backoff of 2s, 5s, 10s, and 30s. A click offers **Retry now**.
- **Unauthorized:** stop automatic retries, show “Authentication required”, and
  open connection settings.
- **Agent disappears:** remove it from selection. Preserve unsent text only if
  the same agent returns; otherwise clear it and explain why.
- **Agent becomes terminal before send:** daemon returns conflict; retain text,
  disable Send, and offer **Open dashboard**.
- **Logs fail:** keep the reply surface available and show “Context unavailable”.
- **Malformed response:** retain the last good snapshot, mark data stale, and
  retry. Never crash the UI because one agent record is invalid.
- **Window/display changes:** clamp the window to a visible display on the next
  layout event.
- **Pet process crashes:** daemon and agents are unaffected.

## Accessibility

- Full keyboard navigation is available after the pet is focused.
- Escape collapses the bubble; Enter submits only while the reply field is
  focused.
- Every animation state has an accessible text label.
- Minimum hit target is 32×32 logical pixels; the reply bubble supports OS text
  scaling.
- The character does not continuously move around the screen.
- Reduced-motion mode replaces animations with static poses and badge changes.
- Sound is supplemental and never the only attention signal.

## Testing

### Pure unit tests

- Snapshot diff emits one event for a genuine status transition.
- Unchanged attention status does not emit repeated events.
- Initial and reconnect snapshots populate attention state without sound events.
- Aggregate priority follows waiting > stuck > limited > error > active > idle.
- Multiple agents of equal priority sort by most recent `updated_at`.
- Read/unread behavior does not mutate daemon status.
- Input validation rejects empty, multiline, control-character, and oversized
  replies.
- Terminal statuses disable reply input.
- Pause-alert logic suppresses presentation but retains unread events.
- Window coordinates clamp to available display bounds.

### HTTP integration tests

- A fleet snapshot is decoded from the real `AgentResponse` shape.
- Reply sends the expected JSON to `/agents/:id/input` exactly once.
- 401, 404, 409, 5xx, timeout, and malformed JSON map to actionable UI errors.
- Logs are fetched only when requested and control sequences are stripped.

### Manual verification

1. Launch the daemon and pet with no agents: pet is idle.
2. Start an agent: pet enters working state within four seconds.
3. Make the agent wait for input: one bubble and unread badge appear.
4. Reply from the pet: the text reaches the tmux session once and the field
   clears only after success.
5. Put two agents into attention states: ordering and switching are correct.
6. Stop the daemon: disconnected state appears and reconnects without an alert
   storm after restart.
7. Move the pet, quit, relaunch, and change displays: position remains visible.
8. Enable reduced motion and mute: no movement or sound occurs, while text and
   badges still communicate state.

## Expected implementation surface

This is a design specification, not an implementation plan. The expected file
surface is recorded only to define boundaries:

| Area | Expected change |
| --- | --- |
| Workspace manifest | Add optional `kaiju-pet` member |
| `crates/kaiju-pet/` | New native companion binary and tests |
| `crates/kaiju-cli` | Optional thin `kaiju pet` launcher |
| Daemon API | No endpoint changes for MVP |
| Core agent model | No changes for MVP |
| Dashboard | No behavior changes for MVP |
| README / verification | Document launch, preferences, and manual checks |

Pet artwork and animation frames are product assets owned by the pet crate.
They must be bundled at compile time so the companion works offline.

## Rollout

1. Ship as an explicitly launched experimental binary on macOS.
2. Keep auto-start disabled until crash/reconnect and notification-noise behavior
   has been validated in daily use.
3. Add login auto-start as an opt-in preference after the experimental period.
4. Evaluate Linux and Windows window adapters after the state, notification,
   and reply workflows are stable.

## Success criteria

1. An attention transition becomes visible in the pet within four seconds under
   normal local operation.
2. An operator can reply to a waiting agent in two actions: open the bubble and
   send the text.
3. The same unchanged status never produces repeated notification sounds or
   bubbles.
4. A reply is delivered at most once and is never retried automatically.
5. Disconnecting, quitting, or crashing the pet has no effect on daemon or agent
   processes.
6. The daemon, CLI, and dashboard can still be built and used without building
   or launching the pet.
7. The companion works offline after installation and makes no non-Kaiju network
   request.

## Accepted limitations and future questions

- MVP is macOS-first; transparent always-on-top behavior on other platforms is
  deferred behind the window adapter.
- Polling may surface an event up to roughly four seconds after the underlying
  terminal change (daemon detection plus pet polling). This is acceptable for
  quick human replies.
- The intended mascot is an **owl** — a quiet, perched sentinel whose natural
  vocabulary (head swivel toward you, wide-eyed alert, ruffled feathers, a single
  wing-flap on completion) maps directly onto the visual-states table. A dog was
  considered and remains a fallback. The initial art style and animation set still
  require a separate visual-design pass; behavior and accessibility do not depend on
  the chosen mascot.
- A future daemon event stream could reduce latency, but is not justified for
  MVP.
- Quick action buttons such as **Approve**, **Continue**, or **Interrupt** should
  be considered only after real reply patterns are observed; free-form input is
  safer and sufficient for the first release.

## Implementation considerations (addendum, 2026-07-16)

This addendum records decisions the design left implicit, grounded in the current
daemon code. It is written for the implementer, not the reader deciding whether to
build the pet.

### Status wire format — pin the contract

The `/agents` handler serializes status with `format!("{:?}", status).to_lowercase()`
(`crates/kaiju-daemon/src/api.rs:228`), **not** serde. The `AgentStatus` enum also
carries `#[serde(rename_all = "snake_case")]` (`crates/kaiju-core/src/agent.rs:40`),
so serde would emit `waiting_for_input`, but the HTTP body emits `waitingforinput`
(no underscore). The pet consumes the HTTP body, so it must parse the **no-underscore**
form. This coupling is fragile: renaming a variant silently changes the wire string
with no compiler error. The pet must own a `parse_status(&str) -> Status` that treats
any unknown string as a non-attention `Unknown` state (never panic), and the pet's
HTTP contract test (see plan Task 2) is the guard. Recommended daemon follow-up
(out of scope for the pet MVP): make `AgentResponse.status` serialize via serde so
there is one canonical form.

### Response shapes the pet decodes

- `GET /agents` → JSON array of `AgentResponse` (`api.rs:189-207`). Fields the pet
  uses: `id`, `agent_type`, `status`, `workspace`, `prompt`, `updated_at` (RFC3339),
  `session_name`. The pet mirrors this as its own `AgentSnapshot` decode struct with
  `#[serde(default)]` on optional fields so an added daemon field never breaks decode.
- `GET /agents/:id/logs` → `{ "logs": "<single string>" }` (`api.rs:510`), the whole
  captured pane, **not** a line array. The pet splits on `\n`, strips ANSI/control
  sequences, and keeps the last five non-empty lines.
- `POST /agents/:id/input` body `{ "text": "..." }` → `{ "status": "sent" }` on 200;
  `409` when the agent is terminal (`api.rs:473`); `404` when the id is unknown.

### Threading and repaint model (egui gotcha)

egui only repaints on input or when explicitly asked. A background poll that arrives
while the window is idle will not show until the next user event unless the worker
calls `egui::Context::request_repaint()` after pushing a snapshot. The model:

- One background worker thread owns the `reqwest::blocking` client (or a small tokio
  runtime) and the poll loop.
- Worker → UI: a `std::sync::mpsc` channel of `WorkerMsg` (latest snapshot, log
  result, send result, connection state). The UI drains it each frame and keeps only
  the newest snapshot.
- UI → worker: an `mpsc` channel of `UiCmd` (`FetchLogs(id)`, `SendReply{id,text}`,
  `RetryNow`, `Shutdown`).
- After every push to the UI channel the worker calls `ctx.request_repaint()`.

### Workspace optionality — concrete mechanism

Adding `crates/kaiju-pet` as a plain workspace member would pull `eframe`/`egui`
into a bare `cargo build` at the repo root, violating goal "daemon/CLI build without
the pet." Fix: add a `default-members` list to the root `Cargo.toml` that includes
the four existing crates but **not** `kaiju-pet`. Then `cargo build` / `cargo test`
at the root skip the GUI crate; `cargo build -p kaiju-pet` builds it explicitly. The
CI ship-gate (fmt+clippy+test) runs the default set plus an explicit `-p kaiju-pet`
build so the pet is still checked without slowing the common path.

### Dependencies (minimize, isolate to the pet crate)

- `eframe` + `egui` (pinned, e.g. `0.28`) — GUI. Transparent frameless always-on-top
  via `egui::ViewportBuilder::default().with_decorations(false).with_transparent(true)
  .with_always_on_top().with_taskbar(false)`.
- `reqwest` (blocking, already a workspace dep) — HTTP.
- `serde` / `serde_json` (workspace) — decode.
- ANSI stripping: hand-rolled state machine over the CSI/OSC grammar in the pet, or a
  single small crate (`strip-ansi-escapes`). Prefer the hand-rolled ~30-line function
  to hold the dependency line; it is trivially unit-testable.
- Sound: play the bundled cue with `afplay` via `std::process::Command` on macOS
  (no audio crate in v1); the window adapter owns this so Linux/Windows can swap it.
- Token storage (`keyring` crate) and OS reduced-motion detection are **deferred**;
  MVP is loopback-only and `reduced_motion` defaults to `false` with a manual toggle.

### "Does not take focus until clicked" — known risk

A frameless always-on-top egui/winit window still activates on click, and true
click-through/no-activation floating windows need platform-specific `NSPanel`
(`NSWindowStyleMask::NonactivatingPanel`) work on macOS. v1 accepts an ordinary
top-level window that comes to front on click; the non-activating behavior is a
window-adapter enhancement, not an MVP blocker. This is called out so it is not
mistaken for a bug during review.

### Error taxonomy

The pet maps transport and HTTP outcomes to a single `PetError` (thiserror) so the UI
switches on intent, not on raw status codes:

| Cause | `PetError` | UI effect |
| --- | --- | --- |
| connect refused / timeout | `Transport` | disconnected state, backoff |
| 401 | `AuthRequired` | stop retries, "Authentication required" |
| 404 on input | `AgentGone` | drop agent from selection |
| 409 on input | `AgentTerminal` | retain text, disable Send, offer dashboard |
| 5xx | `Server` | stale data, retry |
| bad JSON | `Decode` | keep last snapshot, mark stale |
