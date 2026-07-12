# Auto-Continue on Usage Limit — Design

**Date:** 2026-07-12
**Status:** Approved (design), pending implementation plan
**Scope:** Claude Code adapter only (first iteration)

## Problem

When a Claude Code agent hits its usage/session limit mid-run, it stops and
shows a banner like:

```
You've hit your session limit · resets 5:30pm (Asia/Calcutta)
/upgrade or /usage-credits to finish what you're working on.
```

The tmux session stays alive at its prompt, but the work is stalled until the
reset time. Today Kaiju parses this line as `AgentStatus::Error` (terminal,
non-actionable), so the run silently dies and a human must notice and manually
type `continue` after the reset.

We want the daemon to: detect the limit, parse the reset time, and — for agents
that opted in — automatically inject `continue` into the live session once the
limit resets.

## Goals

- Detect the Claude limit banner and extract the reset time + timezone.
- Represent the limited state as a distinct, actionable status.
- For opted-in agents, auto-fire `continue` + Enter at (or just after) the reset
  instant, with no browser tab open.
- Be self-correcting: if the agent is still limited after firing, re-arm.

## Non-goals

- Codex / Gemini limit handling (future iteration; the status + arming machinery
  is generic, only the banner parsing is Claude-specific).
- Configurable resume text (fixed to `continue` this iteration).
- Surviving a daemon restart across the wait window (see Open Questions —
  acceptable for v1; the banner is re-detected on the next poll after restart).

## Chosen approach

**Adapter detects → monitor schedules & fires.** The Claude adapter recognizes
the banner and extracts the reset time; the existing `monitor.rs` loop (already
polls every pane, and has the clock, the store, and tmux access) computes the
reset instant, arms a per-agent timer, and injects `continue` when it elapses.

Rejected alternatives:
- **Separate resume-scheduler loop** — would re-capture and re-parse the same
  panes the monitor already reads (duplicated IO, second source of truth).
- **Frontend `setTimeout` → POST `/input`** — only works while a browser tab is
  awake; useless for an unattended overnight reset.

## Components

### A. `AgentStatus::Limited` (kaiju-core)

New variant in `crates/kaiju-core/src/agent.rs`.

- `is_terminal()` → `false` (the session is alive and will resume).
- `is_active()` → `true` (it's an attention state, like `Stuck`).
- Ordering/labels updated wherever the enum is matched exhaustively
  (serialization, dashboard status maps).

Rationale: distinct from `Error` so (1) the auto-continue logic has a clean
signal to key off, and (2) the dashboard can show "limited · resumes 5:30pm"
instead of a dead "error".

### B. Pure detection + time parsing (kaiju-adapters, no IO, unit-tested)

In `crates/kaiju-adapters/src/claude.rs`:

1. `parse_output` recognizes the banner. Trigger: a line containing `resets`
   together with `session limit` or `usage limit`. On match → status `Limited`.
   This branch is checked **before** the existing `Error:` branch so the limit
   line no longer falls through to `Error`.

2. A pure helper extracts the raw reset fields from the banner:
   `parse_limit_reset(output: &str) -> Option<(String /*time*/, String /*tz*/)>`
   e.g. `("5:30pm", "Asia/Calcutta")`. Regex targets the real format:
   `resets\s+(\d{1,2}(:\d{2})?\s*[ap]m)\s*\(([^)]+)\)` (case-insensitive).

3. A pure time resolver in a new module `crates/kaiju-adapters/src/limit.rs`
   (keeps the timezone math and its `chrono-tz` use isolated and independently
   testable; `claude.rs` calls into it):
   `reset_to_utc(now: DateTime<Utc>, time_str: &str, tz_name: &str) -> Option<DateTime<Utc>>`
   - Parse `time_str` → (hour 0–23, minute).
   - Parse `tz_name` via **chrono-tz** (`tz_name.parse::<Tz>()`); `None` on bad
     zone.
   - Compute today's occurrence of that wall-clock time in `tz`, convert to UTC.
   - If that instant is `<= now` (already passed / clock skew), return `now`
     (fire immediately). We never roll forward a full day: Claude session resets
     are intra-day, and the banner is only present while the reset is imminent.

`ParsedOutput` gains an optional field to carry the resolved instant OR the raw
(time, tz) pair up to the monitor. Decision: carry the **raw (time, tz)** in
`ParsedOutput` and resolve to UTC in the monitor, so the adapter stays free of
"now" (keeps `parse_output` a pure function of its input). Field:
`limit_reset: Option<(String, String)>`.

### C. Opt-in plumbing

- `AgentConfig` (kaiju-core) gains `auto_continue: bool` (default `false`).
- `Settings` (kaiju-daemon) gains `auto_continue: bool`, applied to new configs
  in `Settings::apply` exactly like `isolate`.
- Request bodies for `POST /agents` and `POST /compare` accept `auto_continue`
  (default false); persisted onto the agent so the monitor can read it.
- Dashboard:
  - New-Agent modal: an "Auto-continue when rate-limited" checkbox.
  - Preferences popover: same checkbox bound to the daemon default
    (`loadPrefs`/`savePrefs`, mirroring `pref-isolate`).

### D. Arm & fire (kaiju-daemon monitor)

Add a runtime field `resume_at: Option<DateTime<Utc>>` to the stored agent (not
persisted; runtime scheduling state).

Per poll, in `monitor.rs`, after status is resolved:

- **Arm:** if `status == Limited` and `agent.auto_continue` and
  `resume_at == None` and the parser produced `limit_reset`, compute
  `reset_to_utc(now, time, tz)` and store it as `resume_at`. (Guard on `== None`
  so an armed timer isn't overwritten every poll.)
- **Fire:** if `resume_at` is set and `now >= resume_at`:
  - `TmuxManager::send_keys(session, "continue")` (types `continue` + Enter).
  - Clear `resume_at`; stamp `last_fired: DateTime<Utc>`.
- **Backoff:** do not fire again within `MIN_RETRY_BACKOFF` (~60s) of
  `last_fired`, so a still-limited agent whose reset time resolves to "now"
  can't tight-loop. If the banner persists past the backoff, the arm step
  re-arms with the (possibly new) reset time and it waits again.
- **Disarm:** if `status != Limited` (agent recovered / moved on), clear
  `resume_at`.

The fire decision is factored into a pure function for testing:
`should_fire(resume_at, last_fired, now, backoff) -> bool`.

### E. Dashboard UX (kaiju-daemon assets)

- `dashboard-utils.js`: add `limited` to `ORDER`, `ATTENTION`, and
  `STATUS_LABELS` (label e.g. "limited"). Place it near `waitingforinput`/`stuck`
  in `ORDER` so limited cards sort to the top.
- `dashboard.rs` CSS: `.s-limited` / `.dot.s-limited` / `.agent-card.s-limited`
  styled amber (same family as waiting-for-input), and covered by the `attention`
  treatment so a limited card also pulses / rings when selected.
- Detail panel: when `resume_at` is present on the selected agent, show
  `limited · resumes 5:30pm` (local time from the ISO instant). Requires exposing
  `resume_at` on the `/agents` JSON.

## Data flow

```
tmux pane text
  → ClaudeAdapter::parse_output           (pure) → status=Limited, limit_reset=(t,tz)
  → monitor poll_once                      (IO)
      → resolve_status → Limited
      → arm:  reset_to_utc(now,t,tz)        (pure, chrono-tz) → resume_at
      → fire: should_fire(...)              (pure) → send_keys("continue")
  → store: agent.status, agent.resume_at
  → GET /agents exposes status + resume_at → dashboard renders + auto-continue badge
```

## Error handling

- Unparseable time or unknown timezone → `reset_to_utc` returns `None` → agent
  stays `Limited`, no timer armed (dashboard still shows "limited"; a human can
  intervene). We do **not** invent a fallback delay this iteration (the real
  format is known and targeted); revisit if we see unparsed variants.
- `send_keys` failure is logged and ignored; `resume_at` is cleared and the next
  poll re-arms from the persistent banner, so a transient tmux hiccup self-heals.
- Agent no longer exists / session ended → normal monitor handling; no fire.

## Dependencies

- **chrono-tz** (new, workspace dep) — resolve IANA zone names (`Asia/Calcutta`)
  to UTC. Justified: the banner gives an IANA name with no offset, and there is
  no reasonable hand-rolled substitute. Added to kaiju-adapters (and the
  workspace `[workspace.dependencies]`).

## Testing

Pure unit tests (no IO):
- Banner detection: the real sample → `Limited` + `("5:30pm","Asia/Calcutta")`;
  a normal `Error:` line still → `Error`; a running line unaffected.
- `parse_limit_reset`: `3pm`/`5:30pm`, upper/lower `AM/PM`, extra spacing;
  missing tz / missing time → `None`.
- `reset_to_utc`: known instant for a fixed `now` + `5:30pm` + `Asia/Calcutta`;
  time already past `now` → returns `now`; bad tz → `None`; no-minutes `3pm`.
- `should_fire`: `now < resume_at` → false; `now >= resume_at` → true; within
  `MIN_RETRY_BACKOFF` of `last_fired` → false; `resume_at == None` → false.
- `Settings::apply` sets `auto_continue` on the config like `isolate`.

Existing suites (Rust + `dashboard-utils.test.js`) must stay green; ship gate
(`fmt`, `clippy`, `test`, node tests) runs before merge.

## Open questions / accepted limitations

- **Daemon restart during the wait window:** `resume_at` is runtime-only, so a
  restart forgets the armed timer — but the banner is still on the pane, so the
  next poll re-detects `Limited` and re-arms. Acceptable for v1.
- **Wall-clock ambiguity across DST / day boundary:** intra-day assumption
  documented in `reset_to_utc`; not handled beyond "past → now".
