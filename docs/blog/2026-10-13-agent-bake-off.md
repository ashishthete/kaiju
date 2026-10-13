---
title: "Kaiju: an agent bake-off for your repo"
published: false
description: "Run one coding task across Claude Code, Codex and Gemini at once, each in its own git worktree, then let your tests pick the winner."
tags: rust, ai, opensource, productivity
---

Every few weeks someone tells me a different coding agent is the best one now. They might be right, for their codebase. I wanted to know which one is best at *mine*, and the only honest way to find out is to give them all the same task and check the results.

So I built **Kaiju**: a small Rust daemon that runs one coding task across several AI CLIs at once (Claude Code, Codex, Gemini), each in its own isolated git worktree, then lets **my tests** decide the winner and merges that run back into my branch.

Repo: [github.com/ashishthete/kaiju](https://github.com/ashishthete/kaiju)

## The loop: compare, judge, promote

### Compare

One prompt goes to N agents. Each one runs in its own tmux session, on its own `kaiju/<id>` branch in a separate git worktree. They share no state, so they can't clobber each other, and the dashboard shows the runs side by side with live diffs.

### Judge

Hit **Judge** and a local `claude -p` ranks the runs. Two things keep it honest:

- The runs are **anonymized** as A/B/C, so the judge can't favor a brand.
- If you give it a **test command** (say, `cargo test`), Kaiju runs it in each worktree and the judge weighs pass/fail over how nice the diff looks.

No API key is needed, because it uses the CLI you already have signed in.

### Promote

One click commits the winning run's changes and merges its branch into your workspace. It refuses if your workspace is dirty or the run is still active, and it aborts cleanly on a conflict and tells you which branch to merge by hand.

Every judged bake-off is recorded per repo, so over time the **leaderboard** shows which model wins most often, and whose tests pass most often, for that codebase.

## It is also a fleet manager

The bake-off sits on top of a general tool for running many terminal agents at once:

- **Live dashboard with a real terminal per agent** (xterm.js over a WebSocket). You can type, send Ctrl-C and answer approval prompts from the browser.
- **Reachable from your phone.** Bind to your LAN, pair a device with a QR code, and each device gets its own revocable token.
- **A monitor that tells you when an agent needs you.** It flags `waiting-for-input`, `stuck` and `error`, and can alert by console bell, desktop notification or Slack.
- **A task queue with a bounded pool**, so you can enqueue a backlog and let it work through a few at a time.
- **Adopt an existing Claude Code session** by id and bring it under management.
- **Budget caps.** Set `max_tokens` or `max_cost_usd` and the monitor stops an agent that reaches either.
- **Any other CLI.** An agent type that isn't built in is treated as the executable to run, so `kaiju start -t aider ...` works.

It is local-first: a single daemon binary, no external services, bound to `127.0.0.1` by default.

## And a desktop pet

Kaiju also ships an optional desktop companion for macOS: a small animated blob that floats in the corner of your screen, above every Space and above full-screen apps.

- **It reflects the fleet.** The blob's motion and tint change with what your agents are doing, so you can tell at a glance whether things are running, stuck or broken.
- **It tells you when an agent needs you.** An unread badge appears on the blob. Click it to see what the agent is asking.
- **You can answer from the pet.** Send a one-line reply without opening the dashboard.

The pet is a separate process and is left out of the default build, so the daemon, CLI and dashboard work without it. Quitting or crashing it never affects the daemon or your agents. It is experimental and macOS only.

```bash
cargo build -p kaiju-pet-shell
kaiju pet
```

## Quickstart

You need Rust, `tmux`, `git`, and whichever agent CLIs you want to drive on your `PATH`.

```bash
# Install the CLI
cargo install --path crates/kaiju-cli

# Start the daemon (defaults to 127.0.0.1:7800)
cargo run -p kaiju-daemon

# Open the dashboard
open http://127.0.0.1:7800/
```

For a bake-off, click **Compare task** in the dashboard, enter a workspace and a prompt, and tick the CLIs to run.

For a single agent from the terminal:

```bash
kaiju start --agent-type claude --workspace . --isolate --prompt "fix the failing test"

kaiju list
kaiju logs <id>
kaiju send <id> "yes, apply that"
kaiju diff <id>
```

## How it works

1. A client (the CLI, the dashboard, or any HTTP caller) asks the daemon to create an agent.
2. The daemon picks the matching adapter and launches the agent as the **main process of a detached tmux session**. When the agent exits, the session ends, which gives a clean "completed" signal.
3. A monitor polls each running session every 2 seconds, parses the pane through the adapter, and updates status and metrics.
4. You read status, logs and diffs, reply with `send`, open the live terminal, or attach to tmux directly.

The workspace is layered so the domain never depends on the messy parts:

| Crate | Responsibility |
| --- | --- |
| `kaiju-core` | Domain types and the `Adapter` trait. No IO. |
| `kaiju-adapters` | Per-CLI adapters (Claude, Codex, Gemini) that build commands and parse output. |
| `kaiju-daemon` | HTTP API, dashboard, store, tmux integration, task scheduler and monitor. |
| `kaiju-cli` | The `kaiju` command-line client. |

Adding a new CLI means writing one `Adapter` and registering it. Nothing in the daemon or the CLI changes.

## What it does not do well yet

- **Status and metrics are partly heuristic.** For interactive sessions they come from parsing terminal output, which depends on the CLI version. Use `--batch` when you need exact, machine-readable numbers.
- **Cost is blank unless you supply pricing.** Kaiju ships no rates, because they change. You provide a small JSON file.
- **Bake-off runs are unattended by default.** Each run launches with its CLI's auto-approve flag so it can finish without prompting. The runs are isolated in their own worktrees, but you should know that is what "auto-approve" means before you tick it.
- **Session adoption is Claude only** for now.
- `--isolate` needs a git workspace.

## Try it

If you already pay for more than one coding agent, a bake-off on a real task from your own backlog will tell you more than any benchmark will. Pick a task with a test that currently fails, run it across the CLIs you have, and see which one turns it green.

The code is at [github.com/ashishthete/kaiju](https://github.com/ashishthete/kaiju). Issues and adapter PRs are welcome.
