# Kaiju

**A unified control plane for terminal-based AI coding agents** — Claude Code,
Codex, Gemini CLI, and any custom CLI. Kaiju runs each agent in its own tmux
session, supervises it, and gives you one place to spawn, watch, drive, and
clean up a whole fleet — from a CLI, an HTTP API, or a live web dashboard.

## Highlights

- **One fleet, many agents.** Spawn Claude/Codex/Gemini (or your own CLI) and
  track them all together with live status, runtime, token, and cost metrics.
- **Live web dashboard** with a built-in **interactive terminal** per agent
  (xterm.js over a WebSocket): real-time colored output, and you can type into
  it — Enter, Ctrl-C, arrows — to answer prompts and approvals.
- **Supervision built in.** A background monitor flags `waiting-for-input`,
  `stuck` (output went quiet), `completed`, and `error`, and alerts you once
  (console bell, and optionally Slack).
- **Task queue + bounded pool.** Enqueue a backlog and let Kaiju run N agents at
  a time.
- **Safe parallelism.** `--isolate` runs an agent in its own git worktree on a
  `kaiju/<id>` branch, so concurrent agents in one repo don't collide.
- **Self-contained & local-first.** Single daemon binary, no external services,
  optional bearer-token auth, works offline.

## Requirements

- Rust (stable) and Cargo
- `tmux` and `git` on your `PATH`
- The agent CLIs you want to drive (`claude`, `codex`, `gemini`) on your `PATH`

## Install

```bash
# CLI client -> installs the `kaiju` binary
cargo install --path crates/kaiju-cli

# Daemon (run it from the repo, or `cargo install --path crates/kaiju-daemon`)
cargo run -p kaiju-daemon
```

## Quickstart

```bash
# 1. Start the daemon (defaults to 127.0.0.1:7800)
cargo run -p kaiju-daemon

# 2. Open the dashboard
open http://127.0.0.1:7800/

# 3. Spawn an agent (isolated in its own git worktree)
kaiju start --agent-type claude --workspace . --isolate --prompt "fix the failing test"

# 4. Watch it, answer it, inspect its changes
kaiju list
kaiju logs <id>
kaiju send <id> "yes, apply that"
kaiju diff <id>

# 5. Stop / remove when done
kaiju stop <id>
kaiju remove <id>
```

…or just drive everything from the dashboard: the detail panel has a live
**Terminal** tab (type straight into the agent), plus per-row
interrupt/stop/remove, a "New agent" form, and copy-ID buttons.

**Adopt a session:** to bring an existing Claude Code conversation under Kaiju,
click **Adopt session**, enter the workspace, pick a resumable session from the
list, and Adopt — Kaiju resumes it (`claude --resume <id>`) inside a managed
tmux session, so it shows up in the dashboard (and on paired devices). Close the
original first so two clients don't drive one conversation. Claude only for now;
token metrics don't attribute for adopted sessions yet.

**Drag a file (or image) onto the terminal** to upload it into the agent's
working dir (`.kaiju-uploads/`); Kaiju then types the saved path into the
session, so the agent can read it — handy for images, which can't be streamed
through a terminal.

## CLI

```bash
kaiju start  -t claude -w . --prompt "..."            # spawn interactively
kaiju start  -t claude -w . --isolate --prompt "..."  # own git worktree (kaiju/<id> branch)
kaiju start  -t claude -w . --batch   --prompt "..."  # non-interactive run, exact metrics
kaiju list                                            # fleet overview
kaiju status <id>                                     # status + metrics
kaiju logs   <id>                                     # recent tmux pane output
kaiju diff   <id>                                     # what the agent changed (git diff)
kaiju send   <id> "message"                           # type a reply / approval
kaiju attach <id>                                     # attach to the tmux session
kaiju interrupt <id>                                  # Ctrl-C the session
kaiju stop   <id>                                     # stop
kaiju remove <id>                                     # stop (if running) + clean up
```

Point the CLI at a non-default daemon with `--url` (or `KAIJU_URL`).

Any `-t` value that isn't a built-in (`claude`/`codex`/`gemini`) is treated as a
**custom CLI** — the value is the executable to run, e.g.
`kaiju start -t aider -w . --prompt "..."`. Use `--extra-args` for flags the CLI
needs.

### Task queue

Enqueue a backlog and let a bounded pool work through it (size set by
`KAIJU_CONCURRENCY`, default 2):

```bash
kaiju submit -t claude -w . --isolate --prompt "task 1"
kaiju submit -t claude -w . --isolate --prompt "task 2"
kaiju queue                # queued / running / finished tasks
kaiju cancel <task-id>     # cancel a queued or running task
```

## Configuration

All configuration is via environment variables on the **daemon** (clients only
need `KAIJU_URL` and, if auth is on, `KAIJU_TOKEN`).

| Variable | Purpose | Default |
| --- | --- | --- |
| `KAIJU_PORT` | API / dashboard port | `7800` |
| `KAIJU_HOST` | Bind address. `0.0.0.0` listens on your LAN | `127.0.0.1` |
| `KAIJU_STATE` | Agent state file | `~/.kaiju/state.json` |
| `KAIJU_TASKS` | Task queue file | `~/.kaiju/tasks.json` |
| `KAIJU_WORKTREES` | Base dir for isolated worktrees | `~/.kaiju/worktrees` |
| `KAIJU_CONCURRENCY` | Max agents the pool runs at once | `2` |
| `KAIJU_TOKEN` | Shared bearer token accepted from remote peers (optional — pairing works without it) | — |
| `KAIJU_DEVICES` | Paired-device file (per-device tokens, `0600`) | `~/.kaiju/devices.json` |
| `KAIJU_SLACK_WEBHOOK` | Also post "needs you" alerts to Slack | — |
| `KAIJU_DESKTOP_NOTIFY` | Native desktop notification on "needs you" (`1`/`true`) | off |
| `KAIJU_CLAUDE_BIN` / `KAIJU_CODEX_BIN` / `KAIJU_GEMINI_BIN` | Override an agent's executable (pin a version / use a stub) | found on `PATH` |
| `KAIJU_PRICING` | Token pricing file for cost estimates | `~/.kaiju/pricing.json` |
| `KAIJU_CONFIG` | Global defaults applied to new agents | `~/.kaiju/config.json` |
| `KAIJU_LOGS` | Dir for persisted per-agent logs | `~/.kaiju/logs` |
| `KAIJU_URL` | (client) Daemon URL | `http://127.0.0.1:7800` |

**Metrics & cost:** token counts come from Claude Code's own session transcript
(`~/.claude/projects/<dir>/<session>.jsonl`), so they're exact rather than
scraped. Cost is shown only if you provide pricing — Kaiju ships none, since
rates change. Create `~/.kaiju/pricing.json` mapping model id to per-million
rates (cost is blank for any model not listed; restart to pick up edits):

```json
{
  "claude-opus-4-8":   { "input": 5, "output": 25, "cache_write": 6.25, "cache_read": 0.5 },
  "claude-sonnet-4-6": { "input": 3, "output": 15, "cache_write": 3.75, "cache_read": 0.3 }
}
```

> The rates above are **placeholders** — fill in the current published prices.

**Global defaults:** to avoid repeating the same model / flags / isolation on
every spawn, create `~/.kaiju/config.json` (override with `KAIJU_CONFIG`). Its
values fill in fields a request leaves unset; `default_extra_args` are prepended
to each agent's own args. All keys are optional; restart to pick up edits.

```json
{
  "default_agent_type": "claude",
  "default_model": "claude-opus-4-8",
  "default_extra_args": ["--permission-mode", "acceptEdits"],
  "isolate": true,
  "max_tokens": 2000000,
  "max_cost_usd": 10.0
}
```

`max_tokens` / `max_cost_usd` are optional **budget caps**: the monitor stops an
agent once it reaches either (cost requires pricing to be configured).

**Auth & LAN access:** by default the daemon binds to `127.0.0.1`, so only the
host machine can reach it. To use it from other devices on your network, set
`KAIJU_HOST=0.0.0.0`, which binds **all** network interfaces. On a typical home
machine that means your **local network only** — the router's NAT keeps it off
the internet unless you deliberately port-forward or tunnel. On a cloud VM, or
with an active VPN/Docker bridge, `0.0.0.0` may also expose it on those
interfaces, so prefer binding a specific LAN IP (e.g. `KAIJU_HOST=192.168.1.5`)
there.

The trust model:

- **The host machine is always trusted.** Requests from loopback (`127.0.0.1`)
  need no token — that's your first, authoritative device.
- **Remote devices must authenticate.** A peer that isn't loopback must present
  either the shared `KAIJU_TOKEN` (if set) or a per-device token obtained by
  pairing. Unpaired remote requests are rejected.
- **Pairing:** on the host, open the dashboard → **Preferences (⚙) → Devices →
  Pair a device**. It shows a QR (and a one-time code, valid 10 minutes) that
  encodes `http://<lan-ip>:<port>/pair?code=…`. Scan it from the new device,
  name it, and it receives its own token (stored in the browser like the shared
  token). Each device is listed and individually **revocable**; tokens live in
  `~/.kaiju/devices.json` (`0600`).

Clients send `Authorization: Bearer <token>` (the CLI reads `KAIJU_TOKEN`; the
dashboard stores whichever token it has). `/health`, the dashboard page, the
pairing page, and the vendored assets stay public; the terminal WebSocket
authenticates via a `?token=` query param (browsers can't set headers on a WS
handshake).

**State & restart:** the daemon persists its fleet and reloads on restart,
marking any agent whose tmux session has since ended as stopped.

## HTTP API

| Method | Path | Description |
| ------ | ---- | ----------- |
| GET | `/` | Live fleet dashboard (HTML). |
| GET | `/health` | Liveness check. |
| GET | `/agents` | List all agents. |
| POST | `/agents` | Create an agent (`auto_start`, `isolate` opt-in). |
| POST | `/agents/adopt` | Adopt a session: resume `session_id` in a managed tmux session. |
| GET | `/sessions?workspace=&type=` | Resumable CLI sessions for a workspace (Claude). |
| GET | `/agents/:id` | Get one agent. |
| DELETE | `/agents/:id` | Stop (if running) and remove. |
| POST | `/agents/:id/start` | Start a created agent. |
| POST | `/agents/:id/stop` | Stop a running agent. |
| POST | `/agents/:id/input` | Send a follow-up message / approval. |
| POST | `/agents/:id/interrupt` | Send Ctrl-C to the session. |
| POST | `/agents/:id/files?name=` | Upload a file into the agent's working dir (raw body; drag-drop). |
| GET | `/agents/:id/status` | Status and metrics. |
| GET | `/agents/:id/logs` | Recent tmux pane output. |
| GET | `/agents/:id/diff` | Changes the agent has made (git diff). |
| GET | `/agents/:id/terminal/ws` | Live interactive terminal (WebSocket; `?token=`). |
| GET | `/agents/:id/terminal/size` | Pane dimensions for sizing the terminal. |
| GET | `/assets/xterm.{js,css}` | Vendored terminal renderer (public). |
| POST | `/tasks` | Enqueue a task for the pool. |
| GET | `/tasks` | List queued / running / finished tasks. |
| GET | `/tasks/:id` | Get one task. |
| POST | `/tasks/:id/cancel` | Cancel a queued or running task. |

## Architecture

A layered Rust workspace (see `CLAUDE.md` for the engineering rules):

| Crate | Responsibility |
| ----- | -------------- |
| `kaiju-core` | Domain types (`Agent`, `AgentStatus`, `AgentMetrics`) and the `Adapter` trait. No IO. |
| `kaiju-adapters` | Per-CLI adapters (Claude, Codex, Gemini) that build commands and parse output. |
| `kaiju-daemon` | HTTP API, dashboard, in-memory + persisted store, tmux integration, task scheduler, and the background monitor. |
| `kaiju-cli` | The `kaiju` command-line client. |

Adapters implement a single trait, so adding a new CLI means writing one
`Adapter` and registering it — no daemon or CLI changes.

### How it works

1. A client (CLI, dashboard, or any HTTP caller) asks the daemon to create an agent.
2. The daemon picks the matching adapter and launches the agent as the **main
   process of a detached tmux session** — so the session ending is a clean
   "completed" signal.
3. A monitor polls each running session every 2s, parses the pane through the
   adapter, and updates status + metrics. It marks agents `completed` (session
   ended), `stuck` (output went quiet), and alerts you once on
   waiting/stuck/error.
4. Clients read status/logs/diff, reply with `send`, open the live terminal, or
   attach directly to tmux.

## Testing

```bash
make check     # fmt + clippy (-D warnings) + tests — the full gate
cargo test     # tests only
make smoke     # HTTP API contract against a throwaway daemon
make e2e       # full pipeline with a fake agent (no API keys needed)
```

Unit tests live beside each module; daemon API contract tests are in
`crates/kaiju-daemon/tests/`. See `VERIFICATION.md` for a feature-by-feature
manual checklist (including the browser terminal).

## Limitations

- Status, cost, and token metrics are inferred by parsing CLI terminal output,
  which is heuristic and CLI-version dependent. The waiting-for-input signal is
  judged from the current prompt/menu to stay reliable; completion and cost
  detection may need tuning per CLI. Prefer `--batch` when you need exact,
  machine-readable metrics.
- `--isolate` requires a git workspace; the worktree (branch `kaiju/<id>`) is
  removed when the agent is deleted.
