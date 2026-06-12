# Comparison Judge (LLM-on-diffs) — Design

**Date:** 2026-06-12
**Status:** Approved (brainstorming)

## Goal

Make the side-by-side comparison meaningful by having an LLM judge rank the runs — given the task and each run's diff, return a winner + per-candidate rationale — shown in the comparison view. Anonymized so the judge isn't biased by CLI brand.

## Concept

A **Judge** button in the comparison view gathers each run's git diff + the shared task prompt, anonymizes the candidates as **A/B/C**, asks a local `claude -p` (print mode — no API key) to rank them, and renders the verdict above the columns with a legend revealing which label was which CLI.

## Scope

**v1:** manual Judge button; judge backend = local `claude -p`; diff-only evidence; verdict rendered as text. **Out of scope (noted future):** feeding objective test results to the judge; auto-judging on completion; choosing a different judge backend/model in the UI; promoting the winner.

## Architecture

A new `judge` module holds the pure prompt-building (testable) and the subprocess call. A `POST /compare/judge` endpoint gathers the group, builds the anonymized prompt, runs the judge, and returns the verdict + legend. The UI adds a button + a verdict block.

### Component 1 — `judge` module (`kaiju-daemon/src/judge.rs`)

```rust
pub struct Candidate { pub label: String, pub agent_type: String, pub diff: String }

/// 0 -> "A", 1 -> "B", ... (wraps past 26 as "AA" etc. — only N≤few in practice).
pub fn label_for(index: usize) -> String { ... }

/// Pure: the anonymized judge prompt. Uses ONLY candidate.label (never the CLI
/// name), includes the task, and truncates each diff to a bound so the prompt
/// stays within argv limits. Tested.
pub fn build_prompt(task: &str, candidates: &[Candidate]) -> String { ... }

/// Run the judge: `claude -p <prompt>` in `workspace`, with a timeout. Returns
/// the model's stdout (the verdict). Side effect; not unit-tested.
pub async fn run_judge(workspace: &std::path::Path, prompt: &str) -> Result<String> { ... }
```

- `build_prompt` content: a short rubric — rank candidates best→worst on **correctness, completeness, code quality**; one-line rationale each; end with the winner; be concise. Each candidate block is `### Candidate <label>\n<diff or "(no changes)">`, diffs truncated to ~6000 chars with a `…[truncated]` marker.
- `run_judge`: resolve the claude binary (`KAIJU_CLAUDE_BIN` or `claude`), `tokio::process::Command::new(bin).arg("-p").arg(prompt).current_dir(workspace)`, wrapped in `tokio::time::timeout` (180 s). Non-zero exit / timeout / empty output → `NexusError`.

### Component 2 — `POST /compare/judge` (`api.rs`)

Behind auth. Body `{ "group_id": "..." }`:
1. `agents = store.list()` filtered by `compare_group == group_id`, in stable order (by `created_at`). If `< 2`, `400` ("need at least two runs to judge").
2. For each agent (index i): `label = label_for(i)`, `diff = WorktreeManager::diff(agent.run_dir()).unwrap_or_default()` (empty → "(no changes)"), `agent_type`.
3. `task` = the shared prompt (`agents[0].prompt`, or "" ).
4. `prompt = judge::build_prompt(task, &candidates)`; `verdict = judge::run_judge(workspace, &prompt).await`.
5. Return `200 { "verdict": "<text>", "legend": [{ "label":"A", "agent_type":"claude", "id":"..." }, ...] }`.
6. Judge failure (claude missing / timeout) → `502 { "error": "judge unavailable: ..." }`.

The workspace for `run_judge` = `agents[0].workspace` (the original repo; the judge only reasons over text, cwd is incidental).

### Component 3 — UI (`dashboard.js` + `dashboard.rs`)

In the comparison panel header, a **Judge** button and a `#cmp-verdict` block (hidden until judged). Clicking it:
- disables the button, shows "Judging…" in `#cmp-verdict`.
- `POST /compare/judge { group_id: compareGroup }` via `api()`.
- on success: render the verdict text (escaped, in a `<pre>`/markdown-ish block) plus a legend line mapping `A → claude`, `B → codex`, … so the user can de-anonymize.
- on failure: show the error in the block; re-enable the button.

## Error handling
- `< 2` runs in the group → `400`.
- Unknown group (no matching agents) → `400` (same "need at least two").
- `claude` not found / non-zero / timeout / empty → `502` with the reason; UI shows it.
- Large diffs are truncated in `build_prompt`, so argv stays bounded.

## Testing
- **Pure unit (`judge.rs`):** `label_for` (0→A, 1→B, 25→Z, 26→AA). `build_prompt`: includes the task, uses labels A/B, does NOT contain any `agent_type` string (anonymization invariant), truncates an over-long diff and appends the marker.
- **Daemon integration (`tests/api.rs`, no claude):** `POST /compare/judge` with an unknown/with-<2 group → `400`. (The happy path shells out to `claude`, so it's covered at the gather/validation level, consistent with the suite.)
- **JS:** `node --check`; verdict text is `esc()`'d before innerHTML.

## File touch list
- `crates/kaiju-daemon/src/judge.rs` — new module (label_for, build_prompt, run_judge) + tests.
- `crates/kaiju-daemon/src/lib.rs` — `pub mod judge;`.
- `crates/kaiju-daemon/src/api.rs` — `POST /compare/judge` route + handler + `JudgeRequest`.
- `crates/kaiju-daemon/src/dashboard.rs` — Judge button + verdict block + CSS.
- `crates/kaiju-daemon/assets/dashboard.js` — judge flow.
- Tests alongside.
