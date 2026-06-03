# AgentNexus Roadmap

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
   CLI `agentnexus diff <id>`. Later: `agentnexus pr <id>` to open a PR from the
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
   structured events to `~/.agentnexus/logs/<id>.jsonl` and parses them for
   exact metrics and the final result.

**Acceptance:** diff returns an agent's changes; an idle agent flips to `Stuck`
and alerts; a finished agent reads `Completed`; batch agents report exact
cost/tokens from structured events.

---

## Phase 2B — Visibility (live fleet dashboard)

**Goal:** see the whole fleet at a glance, not via repeated CLI calls.

### Work items

1. **TUI** (`agentnexus watch`): a full-screen table that polls `/agents` and
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
scheduler that keeps up to `NEXUS_CONCURRENCY` agents running and drains the
queue (pure `slots_available`/`task_outcome` + tested), `/tasks` API
(enqueue/list/get/cancel), and `agentnexus submit|queue|cancel`. Still to add:
per-task retries/backoff and timeouts.

### Work items

1. **Task queue:** `POST /tasks` with a prompt, workspace, agent type, and
   concurrency/isolation policy; persisted like agents.
2. **Pool/scheduler:** a worker loop that keeps up to N agents running, pulling
   the next queued task as slots free up. Each task runs isolated by default.
3. **Outcomes:** on task completion, capture the diff/branch and mark the task
   done/failed; retries with backoff for transient failures; per-task timeout.
4. **CLI:** `agentnexus submit`, `agentnexus queue`, `agentnexus cancel`.

Depends on 2A (reliable completion + result capture) and benefits from 2B.

**Acceptance:** submit M tasks with concurrency N; agents drain the queue
honoring N; each finished task has a captured result; failures retry/timeout.

---

## Phase 2D — Productionization

**Goal:** safe for a team and remote use.

### Work items

1. **Auth (done):** bearer-token auth gated on `NEXUS_TOKEN`. Pure `authorized`
   check + middleware (health and dashboard exempt); the CLI sends the token via
   a default header, the dashboard prompts and remembers it. Off when unset.
2. **Remote/multi-user:** bind beyond localhost behind auth + TLS guidance;
   namespace agents/worktrees per user.
3. **Real notifications (Slack done):** `alert` posts to `NEXUS_SLACK_WEBHOOK`
   (fire-and-forget) in addition to the console bell, reusing `should_alert` and
   a pure, tested `alert_message`. OS-level notifications still to add.
4. **Hardening:** structured request logging with ids, rate limits, graceful
   shutdown that optionally stops or detaches running agents.

**Acceptance:** unauthenticated calls are rejected; a waiting agent posts to
Slack; the daemon serves multiple users without cross-talk.

---

## Sequence and gating

2A → 2B → (2C or 2D). 2A is the foundation: a dashboard, queue, or notifications
built on untrustworthy status would amplify the wrong data. Within 2A, result
capture ships first (no architecture change); the batch executor waits on the
structured-output spike. Every phase lands behind `make check`, `make smoke`,
and `make e2e`.
