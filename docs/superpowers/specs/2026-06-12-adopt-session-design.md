# Adopt a Session (Resume by ID) — Design

**Date:** 2026-06-12
**Status:** Approved (brainstorming)

## Goal

Let a user bring an existing CLI conversation under Kaiju's management by **resuming it by session ID** inside a Kaiju-managed tmux session, so it appears in the dashboard and is drivable (and reachable from paired devices) like any other agent.

## Why resume-by-id (not live-process takeover)

Kaiju drives agents entirely through tmux it spawns itself; it cannot attach to an arbitrary live PID, and a session running in a plain terminal has no PTY to hand off. But every CLI session has a persisted, resumable transcript with a session ID. Resuming by ID:

- Works for any prior session — tmux-hosted, plain-terminal, or already closed.
- Reuses Kaiju's existing spawn/start machinery (a normal agent whose launch command is a resume).
- Continues the *same* transcript, so token/cost metrics carry over.

The original process is never touched; Kaiju runs a fresh one continuing the conversation. (The user should close the original first so two clients don't drive one conversation.)

## Scope

**v1: Claude Code only.** Kaiju already knows where Claude persists sessions (`~/.claude/projects/<slug>/<id>.jsonl`; `project_slug()` exists in `claude_transcript.rs`) and how to resume them (`claude --resume <id>`). codex/gemini use different/undocumented session storage, so their adapter methods return "no sessions" and they simply don't appear in the picker. The trait surface is built so they can be added later without touching callers.

**Out of scope (future):** codex/gemini discovery; live-tmux-process takeover; the separate "run one task across multiple CLIs and compare" feature.

## Architecture

A normal Kaiju agent whose launch command happens to be a resume. Discovery and the resume command are adapter responsibilities (Claude-specific knowledge stays in the Claude adapter); the daemon endpoints and UI are thin.

### Component 1 — Session discovery (adapter)

New value type (in `kaiju-core`, alongside other adapter types):

```rust
pub struct SessionInfo {
    pub id: String,            // session id (transcript filename stem)
    pub last_active_unix: i64, // for sorting + display ("2h ago")
    pub first_prompt: String,  // first user message, truncated — human label
}
```

New default trait method on `Adapter` (`kaiju-core/src/adapter.rs`):

```rust
/// Resumable sessions this CLI has recorded for `workspace`, newest first.
/// Default: none (CLI's session storage unknown).
fn list_sessions(&self, _workspace: &std::path::Path) -> Vec<SessionInfo> { vec![] }
```

**Claude impl** (`claude.rs`, helper in `claude_transcript.rs`): resolve `projects/<project_slug(workspace)>/`, list `*.jsonl`, and for each transcript produce a `SessionInfo` — `id` = filename stem, `last_active_unix` = file mtime, `first_prompt` = first user message text (truncated to ~80 chars). Sort newest first. Reuses the existing `project_slug` and projects-root logic (promote `projects_root()` to `pub(crate)` or add a discovery function in `claude_transcript.rs`).

### Component 2 — Resume-by-id command (adapter)

New default trait method:

```rust
/// Command to resume a specific session by id. Default: unsupported.
fn resume_session_command(&self, _config: &AgentConfig, _session_id: &str) -> Option<String> { None }
```

**Claude impl:** `cd <workspace> && claude --resume <id> [--model <m>]` (model from config/default; mirrors the existing `resume_command` shape). The existing `resume_command` (`--continue`, most-recent) is unchanged.

### Component 3 — Endpoints (`kaiju-daemon`)

Both behind the existing auth middleware (so host/loopback and paired devices only).

- `GET /sessions?workspace=<path>&type=<agent_type>` → `200 [{id, last_active, first_prompt}]`. Looks up the adapter for `type`, calls `list_sessions(workspace)`. Unknown/unsupported type → empty list.
- `POST /agents/adopt` body `{ agent_type, workspace, session_id, model? }` →
  1. Validate non-empty `agent_type`, `workspace`, `session_id`.
  2. Build an `AgentConfig` (no prompt; resume drives the TUI).
  3. Create `Agent::new(config)` (fresh `kaiju-<type>-<id8>` session, **no worktree** — resume in place, like `resume_agent_internal`).
  4. Spawn its tmux session running `adapter.resume_session_command(config, session_id)`; if the adapter returns `None`, respond `400 {"error": "<type> does not support resume"}`.
  5. Persist + return `201 AgentResponse`.

Implementation reuses the existing create/start path; the only new launch detail is "use the resume-by-id command instead of `build_command`." A small internal helper (e.g. `adopt_agent_internal`) parallels `start_agent_internal`/`resume_agent_internal`.

### Component 4 — UI (`dashboard.rs` + `dashboard.js`)

An **Adopt** button beside "New agent" opens a modal:

- Workspace path field + agent-type select (default `claude`).
- On workspace entry (blur/Enter), `GET /sessions?workspace=…&type=…`; render a radio list of sessions: `· <relative time> · "<first prompt>" · <short id>`. Empty → "No resumable sessions found for this workspace."
- **Adopt** posts to `/agents/adopt` and opens the new agent's detail panel.
- Hint: *"Close the original session first so two clients don't drive one conversation."*

Follows the existing New-agent modal and `api()` patterns.

### Component 5 — Lifecycle

An adopted agent is a normal agent: terminal mirror, metrics (the resumed session continues the same transcript, so totals carry over), interrupt/stop/delete behave exactly as for any agent. Delete kills the **resumed** tmux session Kaiju created — never the user's original.

## Error handling

- Unsupported `agent_type` for resume → `400` with `{"error": …}` (matches existing API error shape).
- Missing/blank fields → `400`.
- `list_sessions` is best-effort: unreadable projects dir → empty list (never an error), mirroring how `claude_transcript` already tolerates missing/malformed files.
- Spawn failure (tmux) → `500` with the error, like other start paths.

## Testing

- **Adapter unit tests (pure, no tmux):**
  - `list_sessions` for Claude: point `CLAUDE_CONFIG_DIR`/`HOME` at a temp dir with fake `projects/<slug>/<id>.jsonl` files; assert ids, ordering (newest first), and `first_prompt` extraction/truncation. Empty/missing dir → empty vec.
  - `resume_session_command` for Claude: exact command string (with/without model). Non-Claude adapters → `None`.
  - Default trait methods: a custom adapter returns empty `list_sessions` / `None` resume.
- **Endpoint integration tests (`tests/api.rs`, no tmux):**
  - `GET /sessions` with an unsupported type → `200 []`.
  - `POST /agents/adopt` with a blank field → `400`.
  - (Adopt's happy path shells out to tmux, so — consistent with the existing suite excluding tmux endpoints — assert the validation/command-selection logic via the adapter unit tests rather than a live spawn.)

## File touch list (for the plan)

- `crates/kaiju-core/src/adapter.rs` — `SessionInfo` type + two default trait methods.
- `crates/kaiju-adapters/src/claude_transcript.rs` — session-discovery helper (reuse `project_slug`/projects-root).
- `crates/kaiju-adapters/src/claude.rs` — `list_sessions` + `resume_session_command` impls.
- `crates/kaiju-daemon/src/api.rs` — `GET /sessions`, `POST /agents/adopt` routes + handlers.
- `crates/kaiju-daemon/src/server.rs` — `adopt_agent_internal` helper.
- `crates/kaiju-daemon/src/dashboard.rs` — Adopt button + modal HTML/CSS.
- `crates/kaiju-daemon/assets/dashboard.js` — adopt flow (fetch sessions, render, post).
- Tests alongside each.
