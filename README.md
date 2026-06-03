# AgentNexus

A unified control plane for terminal-based AI coding agents (Claude Code, Codex,
Gemini CLI, and custom CLIs). AgentNexus runs each agent inside its own tmux
session, tracks status and metrics, and exposes a small HTTP API plus a CLI to
spawn, observe, and control them.

## Architecture

The workspace follows a layered design (see `CLAUDE.md` for the full rules):

| Crate            | Responsibility                                                                 |
| ---------------- | ------------------------------------------------------------------------------ |
| `nexus-core`     | Domain types (`Agent`, `AgentStatus`, `AgentMetrics`) and the `Adapter` trait. No IO. |
| `nexus-adapters` | Per-CLI adapters (Claude, Codex, Gemini) that build commands and parse output.  |
| `nexus-daemon`   | HTTP API, in-memory store, tmux integration, and the background status monitor. |
| `nexus-cli`      | `agentnexus` command-line client that talks to the daemon.                       |

Adapters implement a single trait, so adding a new CLI means writing one
`Adapter` and registering it — no changes to the daemon or CLI.

## How it works

1. The CLI (or any HTTP client) asks the daemon to create an agent.
2. The daemon picks the matching adapter and launches the agent as the main
   process of a detached tmux session. Because the agent *is* the session
   process, the session ending is a clean "completed" signal.
3. A background monitor polls each running session every 2 seconds, parses the
   pane output through the adapter, and updates status, runtime, token, and cost
   metrics in the store. It marks an agent `completed` when its session ends,
   `stuck` when a running agent stops producing output, and alerts the operator
   once (console bell) on waiting-for-input, error, or stuck.
4. Clients read status/logs/diff, reply with `send`, or attach directly to the
   tmux session.

## Running

Start the daemon (defaults to `127.0.0.1:7800`, override with `NEXUS_PORT`):

```bash
cargo run -p nexus-daemon
```

Then open the live dashboard at <http://127.0.0.1:7800/> — a self-contained page
that polls the API every 2s, color-codes status, and floats agents that need you
(waiting/stuck) to the top.

Agent state is persisted to `~/.agentnexus/state.json` (override with
`NEXUS_STATE`). On restart the daemon reloads its agents and marks any whose
tmux session has since ended as stopped.

To require authentication (recommended before exposing the daemon beyond
localhost), set `NEXUS_TOKEN` on the daemon; clients then send it via
`NEXUS_TOKEN` (the CLI) or are prompted for it (the dashboard). `/health` and
the dashboard page itself stay public. With `NEXUS_TOKEN` unset, auth is off.

Set `NEXUS_SLACK_WEBHOOK` to also receive a Slack message whenever an agent
needs you (waiting for input, stuck, or errored) — in addition to the daemon's
console bell.

Use the CLI:

```bash
cargo run -p nexus-cli -- start --agent-type claude --workspace . --prompt "fix the failing test"
cargo run -p nexus-cli -- start --agent-type claude --workspace . --isolate --prompt "risky refactor"  # own git worktree
cargo run -p nexus-cli -- list
cargo run -p nexus-cli -- status <id>
cargo run -p nexus-cli -- logs <id>
cargo run -p nexus-cli -- diff <id>     # what the agent has changed
cargo run -p nexus-cli -- send <id> "now also update the README"
cargo run -p nexus-cli -- attach <id>
cargo run -p nexus-cli -- stop <id>
```

Or queue a backlog and let a bounded pool work through it (concurrency set by
`NEXUS_CONCURRENCY`, default 2):

```bash
cargo run -p nexus-cli -- submit --agent-type claude --workspace . --isolate --prompt "task 1"
cargo run -p nexus-cli -- submit --agent-type claude --workspace . --isolate --prompt "task 2"
cargo run -p nexus-cli -- queue            # see queued/running/done tasks
cargo run -p nexus-cli -- cancel <task-id>
```

`tmux` must be installed and on `PATH`. Each agent CLI is found on `PATH` by
default; override the executable with `NEXUS_CLAUDE_BIN`, `NEXUS_CODEX_BIN`, or
`NEXUS_GEMINI_BIN` (absolute path) to pin a version or substitute a stub.

## HTTP API

| Method | Path                     | Description                          |
| ------ | ------------------------ | ------------------------------------ |
| GET    | `/`                      | Live fleet dashboard (HTML).         |
| GET    | `/health`                | Liveness check.                      |
| GET    | `/agents`                | List all agents.                     |
| POST   | `/agents`                | Create an agent (`auto_start` opt-in). |
| GET    | `/agents/:id`            | Get one agent.                       |
| DELETE | `/agents/:id`            | Stop (if running) and remove.        |
| POST   | `/agents/:id/start`      | Start a created agent.               |
| POST   | `/agents/:id/stop`       | Stop a running agent.                |
| POST   | `/agents/:id/input`      | Send a follow-up message / approval. |
| POST   | `/agents/:id/interrupt`  | Send Ctrl-C to the session.          |
| POST   | `/tasks`                 | Enqueue a task for the pool.         |
| GET    | `/tasks`                 | List queued/running/finished tasks.  |
| GET    | `/tasks/:id`             | Get one task.                        |
| POST   | `/tasks/:id/cancel`      | Cancel a queued or running task.     |
| GET    | `/agents/:id/status`     | Status and metrics.                  |
| GET    | `/agents/:id/logs`       | Recent tmux pane output.             |
| GET    | `/agents/:id/diff`       | Changes the agent has made (git diff). |

## Testing

```bash
cargo test
```

Unit tests live alongside each module; daemon API contract tests are in
`crates/nexus-daemon/tests/`. For the tmux/git-backed paths there are two
scripts — `make smoke` (API contract against a throwaway daemon) and `make e2e`
(full pipeline using a fake agent, no API keys needed). See `VERIFICATION.md`
for the complete feature-by-feature checklist.

## Status / limitations

- Status and metrics are inferred by parsing CLI terminal output, which is
  heuristic and CLI-version dependent. The waiting-for-input signal is judged
  from the current prompt line to stay reliable, but completion/cost detection
  may need tuning per CLI.
- Pass `--isolate` (or `"isolate": true`) to run an agent in its own git
  worktree on a `nexus/<id>` branch under `~/.agentnexus/worktrees` (override
  with `NEXUS_WORKTREES`), so parallel agents in one repo don't collide. The
  worktree is removed when the agent is deleted. Requires a git workspace.
