# Compare Across CLIs — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Run one prompt across several CLIs at once — each in its own isolated worktree — and review the runs side by side with their live diffs.

**Architecture:** A compare group is N normal isolated agents sharing a `compare_group` id. New surface: one `Agent` field, a `POST /compare` endpoint, a Compare modal, and a side-by-side comparison view. Reuses adapters, `isolate` worktrees, `/agents/:id/diff`, the scheduler, and `renderDiff`.

**Tech Stack:** Rust (core field + daemon endpoint), vanilla-JS dashboard.

---

## File Structure
- `crates/kaiju-core/src/agent.rs` — `compare_group` field + default + test.
- `crates/kaiju-daemon/src/server.rs` — `spawn_compare_group` helper.
- `crates/kaiju-daemon/src/api.rs` — `POST /compare` + `CompareRequest`; `compare_group` on `AgentResponse`.
- `crates/kaiju-daemon/src/dashboard.rs` — Compare button, modal, comparison panel, group badge, CSS.
- `crates/kaiju-daemon/assets/dashboard.js` — compare create + side-by-side view + polling + badge.
- Tests alongside.

---

### Task 1: `compare_group` on `Agent`

**Files:** `crates/kaiju-core/src/agent.rs`

- [ ] **Step 1: Failing test** — append to `#[cfg(test)] mod tests`:

```rust
    #[test]
    fn new_agent_has_no_compare_group_and_roundtrips() {
        let agent = Agent::new(AgentConfig {
            agent_type: AgentType::Claude,
            model: None,
            workspace: std::path::PathBuf::from("/tmp"),
            prompt: None,
            extra_args: vec![],
        });
        assert!(agent.compare_group.is_none());
        let mut g = agent.clone();
        g.compare_group = Some("grp-1".to_string());
        let json = serde_json::to_string(&g).unwrap();
        let back: Agent = serde_json::from_str(&json).unwrap();
        assert_eq!(back.compare_group.as_deref(), Some("grp-1"));
    }
```

- [ ] **Step 2: Run, expect FAIL** — `cargo test -p kaiju-core agent::tests::new_agent_has_no_compare_group` → no field.

- [ ] **Step 3: Add the field + default.** In the `Agent` struct, after the `batch` field, add:

```rust
    /// Groups agents launched together by "Compare task" (a shared run id).
    #[serde(default)]
    pub compare_group: Option<String>,
```

In `Agent::new`, in the constructed `Self { ... }`, after `batch: false,` add:

```rust
            compare_group: None,
```

- [ ] **Step 4: Verify** — `cargo test -p kaiju-core agent::` → pass; `cargo build` (workspace) → clean (the new field with serde default won't break existing constructors, but check any struct-literal `Agent { .. }` in non-test code — there should be none; `Agent::new` is the only constructor).

- [ ] **Step 5: Commit**
```bash
git add crates/kaiju-core/src/agent.rs
git commit -m "feat(core): compare_group field on Agent"
```

---

### Task 2: `POST /compare` endpoint

**Files:** `crates/kaiju-daemon/src/server.rs`, `crates/kaiju-daemon/src/api.rs`, `crates/kaiju-daemon/tests/api.rs`

- [ ] **Step 1: Failing integration tests** — append to `tests/api.rs`:

```rust
#[tokio::test]
async fn compare_rejects_empty_agent_types() {
    let app = build_app(AppState::new());
    let resp = app
        .oneshot(json_request(
            "POST", "/compare",
            serde_json::json!({ "workspace": "/tmp/x", "prompt": "do X", "agent_types": [] }),
        ))
        .await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn compare_rejects_blank_prompt() {
    let app = build_app(AppState::new());
    let resp = app
        .oneshot(json_request(
            "POST", "/compare",
            serde_json::json!({ "workspace": "/tmp/x", "prompt": "", "agent_types": ["claude"] }),
        ))
        .await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn compare_rejects_non_git_workspace() {
    // A temp dir that is not a git repo → 400 before any spawn.
    let dir = std::env::temp_dir().join("kaiju-compare-nongit");
    std::fs::create_dir_all(&dir).unwrap();
    let app = build_app(AppState::new());
    let resp = app
        .oneshot(json_request(
            "POST", "/compare",
            serde_json::json!({ "workspace": dir.to_string_lossy(), "prompt": "do X", "agent_types": ["claude"] }),
        ))
        .await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn agent_response_includes_compare_group_field() {
    use kaiju_core::agent::{Agent, AgentConfig, AgentType};
    let state = AppState::new();
    let agent = Agent::new(AgentConfig {
        agent_type: AgentType::Claude, model: None,
        workspace: std::path::PathBuf::from("/tmp"), prompt: None, extra_args: vec![],
    });
    let id = agent.id.clone();
    state.store.insert(agent);
    let app = build_app(state);
    let resp = app.oneshot(get_request(&format!("/agents/{id}"))).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json.get("compare_group").is_some()); // present, null for a normal agent
}
```

- [ ] **Step 2: Run, expect FAIL** — route 404 / field missing.

- [ ] **Step 3: `spawn_compare_group` in `server.rs`.** After `spawn_started_agent` (or near the other internals), add. Use fully-qualified types to avoid import churn:

```rust
/// Launch one isolated agent per CLI type, all running `prompt` in `workspace`,
/// tagged with a shared compare-group id. Returns (group_id, agent ids).
/// Comparison needs isolation (each run diffs its own worktree), so a non-git
/// workspace is rejected up front. Per-agent start is best-effort: a CLI that
/// fails to launch surfaces as that agent's error, not a whole-group failure.
pub fn spawn_compare_group(
    state: &AppState,
    workspace: &std::path::Path,
    prompt: &str,
    agent_types: &[String],
    model: Option<String>,
) -> Result<(String, Vec<String>)> {
    if !WorktreeManager::is_git_repo(workspace) {
        return Err(NexusError::Git(format!(
            "compare requires a git workspace: {}",
            workspace.display()
        )));
    }
    let group_id = uuid::Uuid::new_v4().to_string();
    let defaults = state.settings.read().expect("settings lock").clone();
    let mut ids = Vec::new();
    for type_str in agent_types {
        let agent_type: kaiju_core::agent::AgentType = type_str.parse().expect("infallible");
        let config = defaults.apply(kaiju_core::agent::AgentConfig {
            agent_type,
            model: model.clone(),
            workspace: workspace.to_path_buf(),
            prompt: Some(prompt.to_string()),
            extra_args: vec![],
        });
        let mut agent = kaiju_core::agent::Agent::new(config);
        agent.isolate = true;
        agent.compare_group = Some(group_id.clone());
        let id = agent.id.clone();
        state.store.insert(agent);
        let _ = start_agent_internal(state, &id);
        ids.push(id);
    }
    Ok((group_id, ids))
}
```
Confirm `WorktreeManager`, `NexusError`, `Result` are in scope in server.rs (they are — used by `prepare_run_dir`/other internals). `uuid` is a daemon dep.

- [ ] **Step 4: `compare_group` on `AgentResponse` (`api.rs`).** Add the field to the struct:

```rust
    pub compare_group: Option<String>,
```
and in `impl From<&kaiju_core::agent::Agent> for AgentResponse`, add to the constructed struct:

```rust
            compare_group: agent.compare_group.clone(),
```

- [ ] **Step 5: `CompareRequest`, route, handler (`api.rs`).** Add request type near the others:

```rust
#[derive(Deserialize)]
pub struct CompareRequest {
    pub workspace: String,
    pub prompt: String,
    pub agent_types: Vec<String>,
    pub model: Option<String>,
}
```
Register the route in `routes()` (near `/agents`):
```rust
        .route("/compare", post(compare))
```
Add the handler:
```rust
/// `POST /compare` — run one prompt across several CLIs, each isolated, grouped.
async fn compare(
    State(state): State<AppState>,
    Json(req): Json<CompareRequest>,
) -> impl IntoResponse {
    use kaiju_core::NexusError;
    if req.workspace.trim().is_empty() || req.prompt.trim().is_empty() || req.agent_types.is_empty() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "workspace, prompt, and at least one agent_type are required",
        ));
    }
    match crate::server::spawn_compare_group(
        &state,
        std::path::Path::new(&req.workspace),
        &req.prompt,
        &req.agent_types,
        req.model,
    ) {
        Ok((group_id, ids)) => {
            let agents: Vec<AgentResponse> = ids
                .iter()
                .filter_map(|id| state.store.get(id).map(|a| AgentResponse::from(&a)))
                .collect();
            Ok((
                StatusCode::CREATED,
                Json(serde_json::json!({ "group_id": group_id, "agents": agents })),
            ))
        }
        Err(e) => {
            let code = match e {
                NexusError::Git(_) => StatusCode::BAD_REQUEST,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            Err(err(code, &e.to_string()))
        }
    }
}
```

- [ ] **Step 6: Verify** — `cargo test -p kaiju-daemon` → all pass (incl. the 4 new). `cargo clippy -p kaiju-daemon` → clean. `cargo build` → clean.

- [ ] **Step 7: Commit**
```bash
git add crates/kaiju-core crates/kaiju-daemon/src/server.rs crates/kaiju-daemon/src/api.rs crates/kaiju-daemon/tests/api.rs
git commit -m "feat(daemon): POST /compare to run a task across CLIs"
```

---

### Task 3: Compare modal (creation UI)

**Files:** `crates/kaiju-daemon/src/dashboard.rs`, `crates/kaiju-daemon/assets/dashboard.js`

- [ ] **Step 1: Button.** After the "Adopt session" button (added by the adopt feature, near the "+ New agent" button at ~line 205), add:
```html
    <button onclick="openCompare()">Compare task</button>
```

- [ ] **Step 2: Modal.** Next to the other `<dialog>` blocks, add:
```html
  <dialog id="comparemodal" class="modal" onclick="if(event.target===this)closeCompare()">
    <div class="modal-head">
      <h2>Compare task across CLIs</h2>
      <button type="button" class="icon" onclick="closeCompare()" title="Close">&times;</button>
    </div>
    <label class="field">
      <span>Workspace path <em>*</em></span>
      <input id="cmp-ws" placeholder="/path/to/repo (git)" autocomplete="off">
    </label>
    <label class="field">
      <span>Prompt <em>*</em></span>
      <textarea id="cmp-prompt" rows="3" placeholder="What should each agent do?"></textarea>
    </label>
    <div class="field">
      <span>Run on</span>
      <label class="check"><input type="checkbox" class="cmp-type" value="claude" checked> claude</label>
      <label class="check"><input type="checkbox" class="cmp-type" value="codex"> codex</label>
      <label class="check"><input type="checkbox" class="cmp-type" value="gemini"> gemini</label>
    </div>
    <div class="note">Each CLI runs the same prompt in its own isolated git worktree, so they don't clobber each other.</div>
    <div class="modal-actions">
      <button type="button" onclick="closeCompare()">Cancel</button>
      <button type="button" class="primary" onclick="submitCompare()">Run comparison</button>
    </div>
  </dialog>
```
(Reuses `.modal`, `.field`, `.check`, `.note`, `.modal-actions`.)

- [ ] **Step 3: JS.** Append to `dashboard.js`:
```javascript
// --- Compare task across CLIs ---

function openCompare() {
  const m = document.getElementById("comparemodal");
  if (typeof m.showModal === "function") m.showModal(); else m.setAttribute("open", "");
}
function closeCompare() {
  const m = document.getElementById("comparemodal");
  if (typeof m.close === "function") m.close(); else m.removeAttribute("open");
}

async function submitCompare() {
  const ws = document.getElementById("cmp-ws").value.trim();
  const prompt = document.getElementById("cmp-prompt").value.trim();
  const types = Array.from(document.querySelectorAll(".cmp-type:checked")).map(c => c.value);
  if (!ws || !prompt || !types.length) { alert("Workspace, prompt, and at least one CLI are required."); return; }
  try {
    const res = await api("/compare", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ workspace: ws, prompt: prompt, agent_types: types }),
    });
    if (!res.ok) { alert((await res.json()).error || "Compare failed."); return; }
    const data = await res.json();
    closeCompare();
    refresh();
    openComparison(data.group_id);   // defined in Task 4
  } catch (e) { alert("Compare failed."); }
}
```
NOTE: `openComparison` is added in Task 4. If implementing Task 3 standalone, temporarily stub `function openComparison(){}` so the JS is valid; Task 4 replaces it. (Recommended: implement Tasks 3 and 4 together before final commit, since the create flow opens the view.)

- [ ] **Step 4: Verify** — `node --check dashboard.js`; `cargo build -p kaiju-daemon`; `cargo test -p kaiju-daemon` (PAGE-references-scripts test stays green).

- [ ] **Step 5: Commit**
```bash
git add crates/kaiju-daemon/src/dashboard.rs crates/kaiju-daemon/assets/dashboard.js
git commit -m "feat(dashboard): Compare-task modal"
```

---

### Task 4: Side-by-side comparison view

**Files:** `crates/kaiju-daemon/src/dashboard.rs`, `crates/kaiju-daemon/assets/dashboard.js`

- [ ] **Step 1: Comparison panel HTML.** Near the agent detail panel (`<div id="detail" ...>`), add a sibling:
```html
  <div id="compare-panel" class="card" hidden>
    <div class="detail-head">
      <span class="id">Comparison</span>
      <span class="path" id="cmp-prompt-label" title=""></span>
      <span class="grow"></span>
      <button onclick="closeComparison()">Close</button>
    </div>
    <div id="cmp-cols" class="cmp-cols"></div>
  </div>
```

- [ ] **Step 2: CSS.** In the `<style>` block, add:
```css
  .cmp-cols { display: flex; gap: 12px; overflow-x: auto; padding: 4px 0; }
  .cmp-col { flex: 1 0 320px; min-width: 320px; display: flex; flex-direction: column;
             border: 1px solid var(--border, #2a2f3a); border-radius: 8px; overflow: hidden; }
  .cmp-col-head { display: flex; align-items: center; gap: 8px; padding: 8px 10px;
                  background: #161b22; font-weight: 600; }
  .cmp-col-head .grow { flex: 1; }
  .cmp-diff { margin: 0; padding: 8px 10px; max-height: 60vh; overflow: auto;
              font-family: var(--mono, monospace); font-size: 12px; white-space: pre; }
```
(If `--border`/`--mono` CSS vars don't exist, use the literal values shown.)

- [ ] **Step 3: JS — render + poll.** Append to `dashboard.js`:
```javascript
// --- Side-by-side comparison view ---

let compareGroup = null;

function openComparison(groupId) {
  compareGroup = groupId;
  document.getElementById("detail").hidden = true;
  document.getElementById("compare-panel").hidden = false;
  renderComparison();
}
function closeComparison() {
  compareGroup = null;
  document.getElementById("compare-panel").hidden = true;
}

// Build/refresh the columns for the current group from the latest /agents data.
async function renderComparison() {
  if (!compareGroup) return;
  let agents;
  try { agents = await (await api("/agents")).json(); } catch (e) { return; }
  const group = agents.filter(a => a.compare_group === compareGroup);
  if (!group.length) { closeComparison(); return; }
  document.getElementById("cmp-prompt-label").textContent = group[0].prompt || "";
  const cols = document.getElementById("cmp-cols");
  cols.innerHTML = group.map(function (a) {
    return '<div class="cmp-col" data-id="' + a.id + '">' +
      '<div class="cmp-col-head">' + esc(a.agent_type) +
      ' <span class="status s-' + a.status + '">' + esc(statusLabel(a.status)) + '</span>' +
      '<span class="grow"></span>' +
      '<button onclick="select(\'' + a.id + '\')">Open</button></div>' +
      '<pre class="cmp-diff" id="cmp-diff-' + a.id + '">Loading…</pre></div>';
  }).join("");
  group.forEach(function (a) {
    api("/agents/" + a.id + "/diff").then(function (r) { return r.json(); }).then(function (d) {
      const pane = document.getElementById("cmp-diff-" + a.id);
      if (pane) pane.innerHTML = d.diff ? renderDiff(d.diff) : "(no changes)";
    }).catch(function () {});
  });
}
```

- [ ] **Step 4: Poll while open + group badge.** In the existing fleet poll/`refresh()` path, after it updates the table, refresh the comparison if open. Find where `refresh()` finishes (it calls `render(agents)`); add right after the render call inside `refresh()`:
```javascript
  if (compareGroup) renderComparison();
```
And in `render(agents)` (the row builder around line 331), add a small badge for grouped agents. In the row template, where the Task cell or an existing cell is built, append a clickable badge when `a.compare_group` is set, e.g. inside the row's actions/status area:
```javascript
      a.compare_group ? '<button class="pill" title="Open comparison" onclick="event.stopPropagation();openComparison(\'' + a.compare_group + '\')">compare</button>' : ''
```
(Place this where it renders cleanly in the existing row — e.g. appended to the Task/Actions cell content. Keep `event.stopPropagation()` so it doesn't also trigger the row's `select`.)

- [ ] **Step 5: Verify** — `node --check dashboard.js`; `node --test dashboard-utils.test.js` (renderDiff still green); `cargo build -p kaiju-daemon`; `cargo test -p kaiju-daemon`.

- [ ] **Step 6: Commit**
```bash
git add crates/kaiju-daemon/src/dashboard.rs crates/kaiju-daemon/assets/dashboard.js
git commit -m "feat(dashboard): side-by-side comparison view + group badge"
```

---

### Task 5: Docs

**Files:** `README.md`

- [ ] **Step 1:** Near the dashboard usage / adopt subsection, add:
```markdown
**Compare across CLIs:** click **Compare task**, enter a workspace + prompt, and
tick the CLIs to run. Each runs the same prompt in its own isolated git worktree;
a comparison view shows the runs side by side with their live diffs. Open any run
to drive its terminal.
```
Add to the HTTP API table:
```markdown
| POST | `/compare` | Run one prompt across CLIs (isolated), grouped for side-by-side review. |
```

- [ ] **Step 2: Commit**
```bash
git add README.md
git commit -m "docs: compare across CLIs"
```

---

## Self-Review Notes
- **Spec coverage:** data model (Task 1), endpoint + validation + group/isolation (Task 2), creation UI (Task 3), side-by-side view + badge + polling (Task 4), docs (Task 5).
- **Type consistency:** `compare_group: Option<String>` flows agent.rs → AgentResponse → JS (`a.compare_group`). `spawn_compare_group(state, &Path, &str, &[String], Option<String>) -> Result<(String, Vec<String>)>` matches the `compare` handler call.
- **Reuse:** `renderDiff`, `statusLabel`, `select`, `refresh`, `/agents/:id/diff`, `WorktreeManager::is_git_repo`, `start_agent_internal`, `defaults.apply` — all existing.
- **No-tmux tests:** validation/precheck covered; the multi-CLI happy path (tmux + worktrees) is exercised manually, consistent with the suite.
- **Tasks 3 & 4 are interdependent** (create flow opens the view) — implement together; commit Task 3 with an `openComparison` stub only if landing separately.
