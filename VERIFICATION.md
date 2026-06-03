# Verifying AgentNexus

Three layers, fastest first. Run them in order — each catches a different class
of problem.

## Layer 1 — Automated unit + contract tests (seconds, no setup)

```bash
make check        # cargo fmt --check + clippy -D warnings + cargo test
```

This compiles everything and runs the in-process test suite: domain logic,
adapter command-building and output parsing (including waiting-for-input and the
menu-prompt case), the registry, the store and its persistence round-trip, the
monitor's metric math, alert transitions, startup reconciliation, worktree
naming, and the HTTP API contract (via in-memory requests — no socket, no tmux).

If this is green, the pure logic is sound. It does **not** exercise real tmux,
git, or agent CLIs — that's Layers 2 and 3.

## Layer 2 — API smoke test (seconds, no agent CLIs)

```bash
./scripts/smoke.sh
```

Boots a throwaway daemon and checks every HTTP endpoint's status-code behavior
with `curl`: health, create, list, get, the 404s, unsupported-type 400, and the
create→stop→input 409 path. Proves the daemon wires up and the routes behave.

## Layer 3 — Full end-to-end with a fake agent (the real proof)

```bash
cargo install --path crates/nexus-cli   # optional, makes it much faster
./scripts/e2e.sh
```

Uses a fake `claude` script on `PATH`, so it needs no API keys yet drives the
entire pipeline deterministically: interactive launch in tmux, status moving to
waiting-for-input, the operator alert, `send` to reply, completion, git-worktree
isolation, persistence across a daemon restart, and worktree cleanup on remove.
This is the closest thing to "verify all features" in one command.

## Layer 4 — Manual check against the real CLIs (one-time validation)

The only thing the fake agent can't validate is whether each real CLI's launch
flag is right. With `claude` / `codex` / `gemini` installed, in a git repo:

```bash
cargo run -p nexus-daemon &                 # watch this terminal for alerts
agentnexus start --agent-type claude --workspace . --prompt "list the files"
agentnexus attach <id>                      # Ctrl-b d to detach
```

Confirm the agent boots into its **interactive** TUI with your prompt loaded
(not a one-shot that exits). If it exits, the launch flag in that adapter's
`build_command` needs adjusting. Repeat for codex and gemini.

## Feature → how to verify → acceptance

| Feature | Layer | Pass condition |
| --- | --- | --- |
| Agent lifecycle / status model | 1 | `cargo test -p nexus-core` green |
| Command building (interactive launch) | 1 | adapter `build_command_*` tests green |
| Output parsing: status/cost/tokens | 1 | adapter `parse_*` tests green |
| Waiting detection (incl. menu prompts) | 1 | `controlling_prompt_line` + `waiting_*` tests green |
| Adapter registry | 1 | `registry` tests green |
| Store + persistence round-trip | 1 | `store` + `persist` tests green |
| Monitor metric math | 1 | `monitor` tests green |
| Operator alert logic | 1 | `notify` `should_alert` tests green |
| Startup reconciliation | 1 | `reconcile` tests green |
| Worktree naming | 1 | `worktree` tests green |
| HTTP API contract | 1 / 2 | `tests/api.rs` green; `smoke.sh` all PASS |
| tmux session lifecycle | 3 | `e2e.sh` reaches waiting + completed |
| Background monitor (live) | 3 | status moves off `starting` on its own |
| Operator alert (live) | 3 | daemon log shows "waiting for your input" |
| Send-input / reply | 3 | reply drives agent to `completed` |
| Git worktree isolation | 3 | `nexus/<id>` worktree appears, then is removed |
| Persistence across restart | 3 | agent still listed after daemon restart |
| Real CLI launch flags | 4 | `attach` shows a live interactive TUI |

## When something fails, where to look

- launch behavior → the adapter's `build_command`
- status / cost / tokens → the adapter's `parse_output`
- waiting/menu detection → `nexus-core/src/adapter.rs`
- alerts → `nexus-daemon/src/notify.rs`
- worktrees → `nexus-daemon/src/worktree.rs` and `server.rs::prepare_run_dir`
- persistence / reconciliation → `persist.rs`, `store.rs`, `reconcile.rs`

## Browser terminal (dashboard)

Open `http://127.0.0.1:7800/`, select an agent, and confirm:
- The detail panel opens on the **Terminal** tab with live, colored output that
  updates within ~250ms of pane changes; **Logs** tab still shows polled text.
- Typing text + Enter, and Ctrl-C / arrows / Esc, reach the agent.
- The **Actions** column copies the full ID and runs interrupt/stop/remove.
- "+ New agent" creates and auto-starts an agent that appears on refresh.
- With `NEXUS_TOKEN` set, the terminal connects with the token and rejects without it.
- Backend smoke (no browser): `GET /assets/xterm.js` → 200 `application/javascript`;
  `GET /agents/:id/terminal/size` → `{"cols":…,"rows":…}`.
