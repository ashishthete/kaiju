# Kaiju Roadmap

v1 (done) is a working control plane: unified spawn, live status/metrics,
waiting-for-input alerts, send-input, git-worktree isolation, persistence, and
restart reconciliation. The phases below build on it in dependency order. Each
phase ends behind a green `make check` before the next begins.

---

## Phase 2A — Trustworthy signals + outcomes

**Goal:** make the data v1 reports dependable, and surface what agents produce.

### The architectural decision to make first

Today, status/cost/tokens are scraped from the tmux pane (heuristic, version
dependent). The robust alternative is **structured output** — e.g. Claude's
`--output-format stream-json`. But structured output is a *non-interactive*
mode, which conflicts with the interactive-supervision model (a live TUI you
attach to). You cannot have both from one process.

Resolution: support **two run modes** per agent:

- `interactive` (current): live TUI in tmux, status by screen-scraping. Best for
  supervision and approvals.
- `batch`: launch the CLI in structured/print mode, capture its JSON event
  stream to a file, parse events for exact status/cost/tokens/result. Best for
  fire-and-forget tasks and reliable metrics.

This needs a short real-CLI spike to confirm each CLI's structured flags and
event schema before building the `batch` executor.

### Work items

1. **Result capture (shipped first — fits v1 as-is).** `git diff` of an agent's
   run directory (its worktree, or the workspace). API `GET /agents/:id/diff`,
   CLI `kaiju diff <id>`. Later: `kaiju pr <id>` to open a PR from the
   agent's `nexus/<id>` branch via `gh`/forge.
2. **Stuck detection (done).** The monitor keeps a per-agent activity record
   (output fingerprint + last-change time). A Running agent with no new output
   for `STUCK_THRESHOLD_SECS` becomes `Stuck` and alerts; it recovers to
   `Running` when output moves again. Pure `resolve_status` helper + tests.
3. **Completion via process exit (done).** The agent runs as the tmux session's
   main process, so the session ending means the agent exited. The monitor marks
   such an agent `Completed`; a manual stop sets `Stopped` first (and leaves the
   active set), so a kill is never misread as completion.
4. **Batch executor (after the spike).** A second execution path that streams
   structured events to `~/.kaiju/logs/<id>.jsonl` and parses them for
   exact metrics and the final result.

**Acceptance:** diff returns an agent's changes; an idle agent flips to `Stuck`
and alerts; a finished agent reads `Completed`; batch agents report exact
cost/tokens from structured events.

---

## Phase 2B — Visibility (live fleet dashboard)

**Goal:** see the whole fleet at a glance, not via repeated CLI calls.

### Work items

1. **TUI** (`kaiju watch`): a full-screen table that polls `/agents` and
   refreshes — status, runtime, cost, and a highlight for agents waiting on you.
   Likely `ratatui`.
2. **Web view (done):** a self-contained dashboard served by the daemon at `/`
   that polls `/agents`, color-codes status, and sorts waiting/stuck agents to
   the top. Clicking an agent opens a detail panel with its live log tail, a
   reply box (`/input`), a diff view, and interrupt/stop actions.
3. **At-a-glance signals:** color by status, sort waiting-for-input to the top,
   show the alert count.

Depends on 2A so the displayed status is trustworthy.

**Acceptance:** one command shows all agents updating live; an agent needing
input is visually obvious; you can read its logs without leaving the view.

---

## Phase 2C — Scale & automation (task queue + agent pool)  — done (core)

**Goal:** submit a backlog and let agents work through it autonomously.

Shipped: `Task`/`TaskSpec`/`TaskStatus` domain, a persisted `TaskStore`, a
scheduler that keeps up to `KAIJU_CONCURRENCY` agents running and drains the
queue (pure `slots_available`/`task_outcome` + tested), `/tasks` API
(enqueue/list/get/cancel), and `kaiju submit|queue|cancel`. Still to add:
per-task retries/backoff and timeouts.

### Work items

1. **Task queue:** `POST /tasks` with a prompt, workspace, agent type, and
   concurrency/isolation policy; persisted like agents.
2. **Pool/scheduler:** a worker loop that keeps up to N agents running, pulling
   the next queued task as slots free up. Each task runs isolated by default.
3. **Outcomes:** on task completion, capture the diff/branch and mark the task
   done/failed; retries with backoff for transient failures; per-task timeout.
4. **CLI:** `kaiju submit`, `kaiju queue`, `kaiju cancel`.

Depends on 2A (reliable completion + result capture) and benefits from 2B.

**Acceptance:** submit M tasks with concurrency N; agents drain the queue
honoring N; each finished task has a captured result; failures retry/timeout.

---

## Phase 2D — Productionization

**Goal:** safe for a team and remote use.

### Work items

1. **Auth (done):** bearer-token auth gated on `KAIJU_TOKEN`. Pure `authorized`
   check + middleware (health and dashboard exempt); the CLI sends the token via
   a default header, the dashboard prompts and remembers it. Off when unset.
2. **Remote/multi-user:** bind beyond localhost behind auth + TLS guidance;
   namespace agents/worktrees per user.
3. **Real notifications (Slack done):** `alert` posts to `KAIJU_SLACK_WEBHOOK`
   (fire-and-forget) in addition to the console bell, reusing `should_alert` and
   a pure, tested `alert_message`. OS-level notifications still to add.
4. **Hardening:** structured request logging with ids, rate limits, graceful
   shutdown that optionally stops or detaches running agents.

**Acceptance:** unauthenticated calls are rejected; a waiting agent posts to
Slack; the daemon serves multiple users without cross-talk.

---

## Phase 3 — Real-time fidelity (streaming + structured + browser terminal)

**Goal:** fix the two real weaknesses of the v1 transport — heuristic
screen-scraping and 2-second polling — without losing what tmux gives us
(attach, persistence, detached runs). Move from "poll and guess" to "stream and
know," and feed a real terminal in the browser.

### Principles

- **Keep tmux for the session.** It already provides attach, persistence, and
  detached execution. We replace only the *capture* mechanism, not the session.
- **Stream, don't poll.** A continuous output feed replaces `capture-pane` every
  2s — lower latency, no 200-line truncation, full scrollback.
- **Prefer structured output.** Where a CLI offers a JSON/stream mode, parse real
  events for exact status/cost/tokens. Fall back to the text parser otherwise.
- **Daemon is always the source of truth.** Agent→server data flows whether or
  not a browser is connected, so persisted state stays correct and alerts
  (bell/Slack) fire while nobody is watching. Browser connections are a separate,
  optional consumer.

### Architecture

Two channels, deliberately decoupled:

1. **agent → server (always on).** `tmux pipe-pane` streams the pane's raw bytes
   to the daemon continuously, or an in-tmux wrapper runs the CLI under a PTY and
   tees bytes (plus structured events where available). The daemon updates the
   store from this stream regardless of any browser.
2. **server → browser (only while a page is open).** A websocket pushes terminal
   bytes and status; the browser sends input and resize. This is where "only when
   the page is open" correctly applies.

### Websocket contract (the seam with the browser-terminal worker)

Endpoint: `GET /agents/:id/stream` (websocket; auth token via query param or
subprotocol, reusing `auth::authorized`). JSON frames:

- server → client:
  - `{ "type": "output", "data": "<utf8 terminal bytes incl. ANSI>" }`
  - `{ "type": "status", "status": "...", "metrics": { ... } }`
- client → server:
  - `{ "type": "input", "data": "..." }`  → tmux `send-keys` / PTY stdin
  - `{ "type": "resize", "cols": N, "rows": N }` → tmux `resize-window` / PTY ioctl

On connect, the server sends a backfill of recent output (e.g. `capture-pane -e`
for the initial paint) before switching to the live `pipe-pane` feed.

**Division of work:** the terminal-emulation effort owns the *frontend* (xterm.js
rendering, the ws client, input/resize) and develops on its **own subtree**. This
phase owns the *daemon* side — the `pipe-pane` stream source, the `/agents/:id/stream`
websocket, and the structured-parser upgrade. The JSON contract above is the only
coupling; agree it first, then both sides can build independently.

### Migration — two independent tracks, REST untouched

Both tracks are **purely additive**: the existing REST API and the `capture-pane`
text parser keep working until each new piece is proven, so nothing breaks
mid-migration. No REST endpoint is removed; the websocket and structured parser
are *additional* consumers/producers, not replacements.

**Track A — structured status, rolled out per adapter (fidelity).**

1. ✅ **Done.** `Adapter::parse_event(line) -> Option<ParsedOutput>` (default
   `None`) plus `claude_events::parse_claude_event` — exact status/cost/tokens
   from Claude `stream-json` (pure, unit-tested). Not yet wired to execution.
2. ✅ **Done.** Batch executor (`batch.rs`): `Agent.batch` / `--batch` runs the
   CLI's `structured_command` non-interactively, streams stdout line-by-line into
   `parse_event`, and updates the store from events (reusing `updated_metrics` and
   `notify`, with cooperative stop). Runs *alongside* the interactive tmux path —
   pick per agent. Tasks adopting `batch` is the small remaining follow-up.
3. **Per-adapter rollout, each with text-scraping fallback:**

   | Adapter | Structured mode | Status |
   | --- | --- | --- |
   | Claude | `-p --output-format stream-json` | parser ✅, executor ✅ |
   | Codex  | the CLI's JSON/exec mode | pending — spike the flag |
   | Gemini | the CLI's JSON mode | pending — spike the flag |
   | Custom | none | stays on the text parser (scraping) |

   The monitor prefers `parse_event` when an agent runs in structured mode, else
   falls back to `parse_output`. An adapter with no structured mode is unchanged,
   so the rollout is safe one CLI at a time.

**Track B — streaming transport + websocket (latency), alongside REST.**

1. `pipe-pane` continuous feed replaces `capture-pane` *polling* into the
   (ANSI-stripped) text parser. No API change. **Note:** `pipe-pane` delivers raw
   bytes *with* ANSI escapes; xterm.js wants those, the text parser does not — so
   stream raw to the browser and strip before parsing.
2. Add `GET /agents/:id/stream` websocket *beside* the REST routes. `/logs`,
   `/status`, `/agents` all remain; existing CLI and dashboard polling keep
   working untouched. The websocket is an extra live consumer.
3. Route browser input/resize over the websocket to `send-keys` / resize.

Tracks A and B are independent — ship structured status without the websocket,
or the websocket without structured status — and neither deletes a REST route.

### Risks / decisions

- ANSI handling split (raw to browser, stripped to parser) — see step 1.
- Websocket auth: present the token on connect; reject otherwise.
- Backpressure: keep a bounded ring buffer of recent output per agent for
  backfill and slow clients.
- Some CLIs may lack a structured mode → those stay on the (now stream-fed,
  ANSI-stripped) heuristic parser.

**Acceptance:** opening an agent in the dashboard shows a live, interactive
terminal with sub-second latency; status/cost come from structured events for
Claude; closing the page does not affect agent state or alerts.

---

## Sequence and gating

2A → 2B → (2C or 2D). 2A is the foundation: a dashboard, queue, or notifications
built on untrustworthy status would amplify the wrong data. Within 2A, result
capture ships first (no architecture change); the batch executor waits on the
structured-output spike. Every phase lands behind `make check`, `make smoke`,
and `make e2e`.
