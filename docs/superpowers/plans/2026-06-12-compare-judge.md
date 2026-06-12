# Comparison Judge (LLM-on-diffs) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`).

**Goal:** A Judge button on the comparison view that asks a local `claude -p` to rank the runs (anonymized A/B/C) on their diffs, returning a winner + rationale shown above the columns.

**Architecture:** A `judge` module (pure prompt build + subprocess), a `POST /compare/judge` endpoint, and a UI button + verdict block. No API key — uses the installed `claude` CLI in print mode.

---

### Task 1: `judge` module

**Files:** create `crates/kaiju-daemon/src/judge.rs`; modify `crates/kaiju-daemon/src/lib.rs` (`pub mod judge;`).

- [ ] **Step 1: Create `judge.rs`** with the pure helpers + subprocess + tests:

```rust
//! LLM judge for the compare feature: rank candidate diffs via `claude -p`.
//! Candidates are anonymized (A/B/C) so the judge isn't biased by CLI brand.

use std::path::Path;

use kaiju_core::{NexusError, Result};

/// One anonymized candidate the judge sees.
pub struct Candidate {
    pub label: String,
    pub agent_type: String,
    pub diff: String,
}

/// Per-candidate diff is capped so the prompt stays within argv limits.
const DIFF_CAP_CHARS: usize = 6000;

/// 0 -> "A", 1 -> "B", ... 25 -> "Z", 26 -> "AA".
pub fn label_for(mut index: usize) -> String {
    let mut s = String::new();
    loop {
        s.insert(0, (b'A' + (index % 26) as u8) as char);
        if index < 26 {
            break;
        }
        index = index / 26 - 1;
    }
    s
}

/// Pure: the anonymized judge prompt. Uses only candidate labels (never the CLI
/// name), includes the task, and truncates each diff.
pub fn build_prompt(task: &str, candidates: &[Candidate]) -> String {
    let mut s = String::from(
        "You are judging candidate solutions to a coding task. Rank them best to \
         worst on correctness, completeness, and code quality. Give a one-line \
         rationale for each candidate, then name the winner. Be concise.\n\n## Task\n",
    );
    s.push_str(task);
    s.push_str("\n\n## Candidates\n");
    for c in candidates {
        s.push_str(&format!("\n### Candidate {}\n", c.label));
        let truncated: String = c.diff.chars().take(DIFF_CAP_CHARS).collect();
        s.push_str(&truncated);
        if truncated.len() < c.diff.len() {
            s.push_str("\n…[truncated]");
        }
        s.push('\n');
    }
    s
}

/// Run the judge: `claude -p <prompt>` in `workspace`, with a timeout. Returns
/// the model's stdout (the verdict). Side effect; not unit-tested.
pub async fn run_judge(workspace: &Path, prompt: &str) -> Result<String> {
    let bin = std::env::var("KAIJU_CLAUDE_BIN")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "claude".to_string());
    let fut = tokio::process::Command::new(&bin)
        .arg("-p")
        .arg(prompt)
        .current_dir(workspace)
        .output();
    let out = tokio::time::timeout(std::time::Duration::from_secs(180), fut)
        .await
        .map_err(|_| NexusError::Adapter("judge timed out".to_string()))?
        .map_err(|e| NexusError::Adapter(format!("judge failed to start ({bin}): {e}")))?;
    if !out.status.success() {
        return Err(NexusError::Adapter(format!(
            "judge exited with error: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() {
        return Err(NexusError::Adapter("judge returned no output".to_string()));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_sequential() {
        assert_eq!(label_for(0), "A");
        assert_eq!(label_for(1), "B");
        assert_eq!(label_for(25), "Z");
        assert_eq!(label_for(26), "AA");
    }

    #[test]
    fn prompt_is_anonymized_and_has_task() {
        let cands = vec![
            Candidate { label: "A".into(), agent_type: "claude".into(), diff: "+ fn a() {}".into() },
            Candidate { label: "B".into(), agent_type: "codex".into(), diff: "+ fn b() {}".into() },
        ];
        let p = build_prompt("add a function", &cands);
        assert!(p.contains("add a function"));
        assert!(p.contains("### Candidate A"));
        assert!(p.contains("### Candidate B"));
        // The CLI names must NOT leak into the judge prompt.
        assert!(!p.contains("claude"));
        assert!(!p.contains("codex"));
    }

    #[test]
    fn long_diff_is_truncated() {
        let big = "x".repeat(DIFF_CAP_CHARS + 500);
        let cands = vec![Candidate { label: "A".into(), agent_type: "claude".into(), diff: big }];
        let p = build_prompt("t", &cands);
        assert!(p.contains("…[truncated]"));
    }
}
```

Add `pub mod judge;` to `lib.rs` (alphabetical — after `files`/before `logstore`).

- [ ] **Step 2: Verify** — `cargo test -p kaiju-daemon judge::` → 3 pass; `cargo build -p kaiju-daemon` → clean (note: `run_judge` is unused until Task 2 — a dead_code warning is expected and resolved by Task 2).

- [ ] **Step 3: Commit**
```bash
git add crates/kaiju-daemon/src/judge.rs crates/kaiju-daemon/src/lib.rs
git commit -m "feat(daemon): judge module — anonymized claude -p ranking of diffs"
```

---

### Task 2: `POST /compare/judge` endpoint

**Files:** `crates/kaiju-daemon/src/api.rs`, `crates/kaiju-daemon/tests/api.rs`

- [ ] **Step 1: Failing test** — append to `tests/api.rs`:
```rust
#[tokio::test]
async fn judge_rejects_group_with_fewer_than_two_runs() {
    let app = build_app(AppState::new());
    let resp = app
        .oneshot(json_request("POST", "/compare/judge", serde_json::json!({ "group_id": "nope" })))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
```

- [ ] **Step 2: Run, expect FAIL** (route 404).

- [ ] **Step 3: Add `JudgeRequest`, route, handler** (`api.rs`):
```rust
#[derive(Deserialize)]
pub struct JudgeRequest {
    pub group_id: String,
}
```
Route in `routes()` (near `/compare`): `.route("/compare/judge", post(compare_judge))`
Handler:
```rust
/// `POST /compare/judge` — rank a compare group's runs with an LLM judge.
async fn compare_judge(
    State(state): State<AppState>,
    Json(req): Json<JudgeRequest>,
) -> impl IntoResponse {
    let mut agents: Vec<_> = state
        .store
        .list()
        .into_iter()
        .filter(|a| a.compare_group.as_deref() == Some(req.group_id.as_str()))
        .collect();
    agents.sort_by_key(|a| a.created_at);
    if agents.len() < 2 {
        return Err(err(StatusCode::BAD_REQUEST, "need at least two runs to judge"));
    }
    let task = agents[0].prompt.clone().unwrap_or_default();
    let workspace = agents[0].workspace.clone();
    let candidates: Vec<crate::judge::Candidate> = agents
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let diff = crate::worktree::WorktreeManager::diff(a.run_dir()).unwrap_or_default();
            crate::judge::Candidate {
                label: crate::judge::label_for(i),
                agent_type: a.agent_type.to_string(),
                diff: if diff.trim().is_empty() { "(no changes)".to_string() } else { diff },
            }
        })
        .collect();
    let legend: Vec<serde_json::Value> = agents
        .iter()
        .enumerate()
        .map(|(i, a)| {
            serde_json::json!({ "label": crate::judge::label_for(i), "agent_type": a.agent_type.to_string(), "id": a.id })
        })
        .collect();
    let prompt = crate::judge::build_prompt(&task, &candidates);
    match crate::judge::run_judge(&workspace, &prompt).await {
        Ok(verdict) => Ok(Json(serde_json::json!({ "verdict": verdict, "legend": legend }))),
        Err(e) => Err(err(StatusCode::BAD_GATEWAY, &format!("judge unavailable: {e}"))),
    }
}
```
Confirm `state.store.list()` returns `Vec<Agent>` (it's used by monitor/reconcile) and `Agent` has `created_at`, `run_dir()`, `workspace`, `agent_type`, `prompt`, `compare_group`, `id`.

- [ ] **Step 4: Verify** — `cargo test -p kaiju-daemon` (all pass incl. new), `cargo clippy -p kaiju-daemon` (clean — judge dead_code gone), `cargo build` clean.

- [ ] **Step 5: Commit**
```bash
git add crates/kaiju-daemon/src/api.rs crates/kaiju-daemon/tests/api.rs
git commit -m "feat(daemon): POST /compare/judge endpoint"
```

---

### Task 3: Judge UI

**Files:** `crates/kaiju-daemon/src/dashboard.rs`, `crates/kaiju-daemon/assets/dashboard.js`

- [ ] **Step 1: Button + verdict block** — in the `#compare-panel` `.detail-head`, add a Judge button before the Close button, and a verdict block before `#cmp-cols`:
```html
      <button id="cmp-judge-btn" onclick="judgeComparison()">Judge</button>
      <button onclick="closeComparison()">Close</button>
    </div>
    <div id="cmp-verdict" hidden></div>
    <div id="cmp-cols" class="cmp-cols"></div>
```

- [ ] **Step 2: CSS** — in `<style>`:
```css
  #cmp-verdict { margin: 0 0 12px; padding: 10px 12px; border: 1px solid var(--border);
                 border-radius: 8px; background: var(--surface-2); }
  #cmp-verdict[hidden] { display: none; }
  .cmp-legend { font-size: 12px; color: var(--muted); margin-bottom: 6px; }
  .cmp-verdict-text { margin: 0; white-space: pre-wrap; font-size: 13px; }
```

- [ ] **Step 3: JS** — append to `dashboard.js`:
```javascript
async function judgeComparison() {
  if (!compareGroup) return;
  const btn = document.getElementById("cmp-judge-btn");
  const box = document.getElementById("cmp-verdict");
  btn.disabled = true;
  box.hidden = false;
  box.innerHTML = '<span class="spinner"></span> Judging…';
  try {
    const res = await api("/compare/judge", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ group_id: compareGroup }),
    });
    if (!res.ok) { box.textContent = (await res.json()).error || "Judge failed."; btn.disabled = false; return; }
    const d = await res.json();
    const legend = (d.legend || []).map(function (l) { return l.label + " = " + esc(l.agent_type); }).join("  ·  ");
    box.innerHTML = '<div class="cmp-legend">' + legend + '</div>' +
      '<pre class="cmp-verdict-text">' + esc(d.verdict) + '</pre>';
  } catch (e) { box.textContent = "Judge failed."; }
  btn.disabled = false;
}
```
And in `openComparison(groupId)`, reset the verdict on open — add after showing the panel:
```javascript
  document.getElementById("cmp-verdict").hidden = true;
```

- [ ] **Step 4: Verify** — `node --check dashboard.js`; `node --test dashboard-utils.test.js`; `cargo build -p kaiju-daemon`; `cargo test -p kaiju-daemon`.

- [ ] **Step 5: Commit**
```bash
git add crates/kaiju-daemon/src/dashboard.rs crates/kaiju-daemon/assets/dashboard.js
git commit -m "feat(dashboard): Judge button + verdict in comparison view"
```

---

### Task 4: Docs

**Files:** `README.md`

- [ ] **Step 1:** Extend the "Compare across CLIs" paragraph:
```markdown
Hit **Judge** in the comparison view to have a local `claude -p` rank the runs
(anonymized) with a winner + rationale — no API key needed.
```
Add API row:
```markdown
| POST | `/compare/judge` | LLM-rank a compare group's runs (anonymized) via local `claude -p`. |
```

- [ ] **Step 2: Commit**
```bash
git add README.md
git commit -m "docs: comparison judge"
```

---

## Self-Review Notes
- **Spec coverage:** module (T1), endpoint + validation + 502 mapping (T2), UI button/verdict/anonymized legend (T3), docs (T4).
- **Anonymization invariant:** `build_prompt` uses only labels; a unit test asserts no CLI name leaks into the prompt. The legend (which de-anonymizes) is returned separately for the UI, never sent to the judge.
- **Type consistency:** `Candidate {label, agent_type, diff}`, `label_for`, `build_prompt`, `run_judge` signatures match the handler's use. `verdict`/`legend` JSON matches the JS.
- **No-claude tests:** the `<2 runs` guard is unit-testable; the happy path shells to `claude`, covered at gather/validation level (consistent with the suite). Judge-missing → 502, surfaced in the UI.
