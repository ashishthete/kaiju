# Compare Across CLIs — Design

**Date:** 2026-06-12
**Status:** Approved (brainstorming)

## Goal

Run one task (prompt) across several CLIs at once — each in its own isolated git worktree so they never clobber each other — and review the results side by side.

## Concept

A "Compare task" action takes a prompt + workspace + a set of CLIs (claude/codex/gemini). It spawns one **isolated** agent per CLI, all running the same prompt, tagged with a shared **compare group id**. A dedicated comparison view shows the group's runs in columns — each with its status and live git diff — so you can eyeball them against each other.

Reuses what already exists: the adapters, `isolate` worktrees, the `/agents/:id/diff` endpoint, and the scheduler. The genuinely new surface is one `Agent` field, one creation endpoint, a modal, and the side-by-side view.

## Scope

**v1:** spawn N isolated agents (one per selected CLI) with a shared group, plus a side-by-side comparison view (type · status · diff · open-terminal link).

**Out of scope (future):** an LLM judge that scores the runs; auto-promoting/merging a winner's branch; running the *same* CLI multiple times for variance.

## Architecture

A compare group is just N normal isolated agents sharing a `compare_group` id. No new store, no new agent lifecycle — they start, run, finish, and are deletable exactly like any isolated agent.

### Component 1 — Data model (`kaiju-core`)

Add to `Agent` (agent.rs), alongside `isolate`/`batch`:

```rust
    /// Groups agents launched together by "Compare task" (a shared run id).
    #[serde(default)]
    pub compare_group: Option<String>,
```

`Agent::new` initializes it to `None` (like `isolate`). It is set after construction by the compare handler. `AgentConfig` is unchanged.

### Component 2 — Creation endpoint (`kaiju-daemon`)

`POST /compare` (behind auth):

```
{ "workspace": "/path", "prompt": "do X", "agent_types": ["claude","codex","gemini"], "model": null }
```

- Validate: non-empty `workspace`, non-empty `prompt`, ≥1 `agent_types`.
- Comparison requires a **git** workspace (each run needs an isolated worktree to diff); reject with 400 if `workspace` is not a git repo (reuse `WorktreeManager::is_git_repo`).
- Generate `group_id` (uuid). For each type: build an `AgentConfig` (with the shared prompt + workspace + applied Preferences defaults), `Agent::new`, set `isolate = true` and `compare_group = Some(group_id)`, insert, and start via `start_agent_internal`. Collect the created `AgentResponse`s.
- Return `201 { "group_id": "...", "agents": [AgentResponse, ...] }`.

A server helper `spawn_compare_group(state, workspace, prompt, agent_types, model) -> Result<(String, Vec<String>)>` holds the loop (parallels `spawn_started_agent`). On a per-agent start failure, the partial group is still returned with whatever started; the error is surfaced per agent (best-effort — a failed CLI shows as an errored/stopped agent, not a 500 for the whole group), **unless** the git-repo precheck fails (then 400 before any spawn).

### Component 3 — `AgentResponse` (`api.rs`)

Add `pub compare_group: Option<String>` to `AgentResponse`, populated from the agent. This lets the dashboard group agents from the existing `/agents` poll — no extra GET endpoint needed.

### Component 4 — UI: Compare modal (`dashboard.rs` + `dashboard.js`)

A **"Compare task"** button beside "New agent" / "Adopt session" opens a modal:

- Workspace path + prompt (textarea) + a checkbox per CLI (claude/codex/gemini, claude checked by default) + optional model.
- Submit → `POST /compare` → open the comparison view for the returned `group_id`.

### Component 5 — UI: Side-by-side comparison view (`dashboard.js` + `dashboard.rs`)

A comparison panel (sibling to the detail panel) that renders a group:

- Header: the shared prompt + workspace.
- One **column per agent** in the group: agent type, status badge, a **diff pane** (fetched from `/agents/:id/diff`, colored via the existing `renderDiff` helper), and an "Open" button that opens that agent's detail panel/terminal (`select(id)`).
- Polls while any column is still running (reuses the existing poll cadence): re-fetch the group's agents from the latest `/agents` data and refresh each diff.
- Entry points: a small **group badge** on fleet rows that belong to a compare group; clicking it opens the comparison view. (The compare modal also opens it directly on creation.)

Layout: CSS grid/flex columns that scroll horizontally if there are many; each diff pane scrolls vertically. Reuses existing `.card`, status-badge, and diff styles.

## Lifecycle

Each run is a normal isolated agent: terminal, metrics, interrupt/stop/delete all work per agent. Deleting an agent removes its worktree as today. There is no group-level delete in v1 (delete the runs individually); the group is just a shared tag.

## Error handling

- Non-git workspace → `400` before spawning anything.
- Missing prompt/workspace or empty `agent_types` → `400`.
- A single CLI failing to start doesn't fail the whole group — it surfaces as that agent's error/stopped status; the group view still shows the others.
- Unknown `agent_type` strings become custom CLI types (consistent with `create_agent`); they simply run that binary.

## Testing

- **Core:** `Agent` round-trips `compare_group` through serde; `Agent::new` defaults it to `None`.
- **Daemon integration (`tests/api.rs`, no tmux):**
  - `POST /compare` with empty `agent_types` → `400`.
  - `POST /compare` with a missing prompt → `400`.
  - `POST /compare` with a non-git workspace → `400` (use a temp non-git dir).
  - `AgentResponse` serializes `compare_group` (assert the field is present/null on a normal agent).
  - (The happy path spawns tmux + worktrees, so — like the rest of the suite — it's covered at the validation/precheck level, not a live multi-CLI spawn.)
- **JS:** `dashboard-utils.js` already unit-tests `renderDiff`; the compare view reuses it. `node --check` for syntax.

## File touch list (for the plan)

- `crates/kaiju-core/src/agent.rs` — `compare_group` field + default in `new` + a serde round-trip test.
- `crates/kaiju-daemon/src/server.rs` — `spawn_compare_group` helper.
- `crates/kaiju-daemon/src/api.rs` — `POST /compare` route + handler + `CompareRequest`; `compare_group` on `AgentResponse`.
- `crates/kaiju-daemon/src/dashboard.rs` — Compare button, compare modal, comparison panel, group badge, CSS.
- `crates/kaiju-daemon/assets/dashboard.js` — compare create flow + side-by-side view + polling + badge wiring.
- Tests alongside each.
