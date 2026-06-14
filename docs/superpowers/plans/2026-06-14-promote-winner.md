# Promote a Winner — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`).

**Goal:** Act on a comparison result — promote a run's changes (its isolated `kaiju/<short>` worktree branch) into the workspace, from a Promote button in the comparison view.

**Architecture:** A run's changes are uncommitted working-tree edits in its worktree on branch `kaiju/<short>` (`branch_name`). Promote = commit those changes on that branch, then merge it into the workspace's current branch. New git helpers live in `worktree.rs`; `promote_agent_internal` orchestrates; `POST /agents/:id/promote` exposes it; the compare UI gets a per-column Promote button.

**Safety (never leave a broken state):**
- Run must be **isolated** with a live worktree, else `400`.
- If the **workspace has uncommitted changes**, refuse before touching anything → `409` ("commit or stash first").
- On **merge conflict**, `git merge --abort` and return `409` with the branch name to merge manually.
- "Nothing to promote" (run made no changes) → `400`.

**Out of scope (future):** opening a PR (`gh`) instead of a local merge; auto-promoting the judge's winner.

---

### Task 1: Git helpers + promote endpoint

**Files:** `crates/kaiju-daemon/src/worktree.rs`, `crates/kaiju-daemon/src/server.rs`, `crates/kaiju-daemon/src/api.rs`, `crates/kaiju-daemon/tests/api.rs`

- [ ] **Step 1: `worktree.rs` helpers.** Add a small private `git` helper (mirroring the closure in `diff`) and three methods + an enum to `impl WorktreeManager` (and a `MergeOutcome` enum near the top):

```rust
/// Outcome of merging a run's branch into the workspace.
pub enum MergeOutcome {
    Merged,
    AlreadyUpToDate,
    Conflict,
}
```
Inside `impl WorktreeManager`:
```rust
    /// True if the working tree at `dir` has no staged/unstaged/untracked changes.
    pub fn is_clean(dir: &Path) -> Result<bool> {
        let out = Command::new("git")
            .args(["-C", &dir.display().to_string(), "status", "--porcelain"])
            .output()
            .map_err(NexusError::Io)?;
        Ok(out.status.success() && out.stdout.is_empty())
    }

    /// Stage everything and commit in `worktree`. Returns whether a commit was
    /// actually made (`false` when there was nothing to commit — not an error).
    pub fn commit_all(worktree: &Path, message: &str) -> Result<bool> {
        let dir = worktree.display().to_string();
        let add = Command::new("git")
            .args(["-C", &dir, "add", "-A"])
            .output()
            .map_err(NexusError::Io)?;
        if !add.status.success() {
            return Err(NexusError::Git(format!(
                "git add failed: {}",
                String::from_utf8_lossy(&add.stderr).trim()
            )));
        }
        let commit = Command::new("git")
            .args(["-C", &dir, "commit", "-m", message])
            .output()
            .map_err(NexusError::Io)?;
        // A non-zero exit here is the "nothing to commit" case — not an error.
        Ok(commit.status.success())
    }

    /// Merge `branch` into the current branch at `workspace` (a `--no-ff` merge).
    /// On conflict, aborts so the workspace is left clean, and reports `Conflict`.
    pub fn merge_branch(workspace: &Path, branch: &str, message: &str) -> Result<MergeOutcome> {
        let dir = workspace.display().to_string();
        let out = Command::new("git")
            .args(["-C", &dir, "merge", "--no-ff", "-m", message, branch])
            .output()
            .map_err(NexusError::Io)?;
        if out.status.success() {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if stdout.contains("Already up to date") {
                Ok(MergeOutcome::AlreadyUpToDate)
            } else {
                Ok(MergeOutcome::Merged)
            }
        } else {
            // Most likely a conflict; abort to restore a clean tree either way.
            let _ = Command::new("git")
                .args(["-C", &dir, "merge", "--abort"])
                .output();
            Ok(MergeOutcome::Conflict)
        }
    }
```
(Confirm `Command`, `Path`, `NexusError`, `Result` are imported at the top of `worktree.rs` — they are, used by `create`/`diff`.)

- [ ] **Step 2: `server.rs` — `promote_agent_internal`.** Add a `PromoteOutcome` enum and the orchestrator (place near `resume_agent_internal`):
```rust
/// Result of promoting a run into its workspace.
pub enum PromoteOutcome {
    Merged { branch: String },
    NothingToPromote,
    WorkspaceDirty,
    Conflict { branch: String },
}

/// Commit an isolated run's changes on its branch and merge that branch into the
/// workspace's current branch. Non-destructive on failure: refuses a dirty
/// workspace up front and aborts a conflicting merge.
pub fn promote_agent_internal(state: &AppState, id: &str) -> Result<PromoteOutcome> {
    let agent = state
        .store
        .get(id)
        .ok_or_else(|| NexusError::AgentNotFound(id.to_string()))?;
    let worktree = match (&agent.isolate, &agent.worktree_path) {
        (true, Some(p)) if p.exists() => p.clone(),
        _ => {
            return Err(NexusError::Git(
                "only an isolated run with a live worktree can be promoted".to_string(),
            ))
        }
    };
    // Refuse before changing anything if the workspace isn't clean.
    if !WorktreeManager::is_clean(&agent.workspace)? {
        return Ok(PromoteOutcome::WorkspaceDirty);
    }
    let branch = worktree::branch_name(&agent.id);
    let short = &agent.id[..agent.id.len().min(8)];
    let made = WorktreeManager::commit_all(
        &worktree,
        &format!("kaiju: promote {} run {short}", agent.agent_type),
    )?;
    match WorktreeManager::merge_branch(
        &agent.workspace,
        &branch,
        &format!("kaiju: promote run {short}"),
    )? {
        MergeOutcome::Merged => Ok(PromoteOutcome::Merged { branch }),
        MergeOutcome::AlreadyUpToDate => {
            if made {
                Ok(PromoteOutcome::Merged { branch })
            } else {
                Ok(PromoteOutcome::NothingToPromote)
            }
        }
        MergeOutcome::Conflict => Ok(PromoteOutcome::Conflict { branch }),
    }
}
```
Add `use crate::worktree::MergeOutcome;` (or reference it fully-qualified) and ensure `WorktreeManager`, `worktree`, `NexusError`, `Result` are in scope (they are).

- [ ] **Step 3: `api.rs` — route + handler.** Register `.route("/agents/:id/promote", post(promote_agent))` (near the other `/agents/:id/*` routes). Handler:
```rust
/// `POST /agents/:id/promote` — merge a run's branch into its workspace.
async fn promote_agent(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    use crate::server::PromoteOutcome;
    use kaiju_core::NexusError;
    match crate::server::promote_agent_internal(&state, &id) {
        Ok(PromoteOutcome::Merged { branch }) => {
            Ok(Json(serde_json::json!({ "status": "merged", "branch": branch })))
        }
        Ok(PromoteOutcome::NothingToPromote) => {
            Err(err(StatusCode::BAD_REQUEST, "nothing to promote — the run made no changes"))
        }
        Ok(PromoteOutcome::WorkspaceDirty) => Err(err(
            StatusCode::CONFLICT,
            "workspace has uncommitted changes — commit or stash them first",
        )),
        Ok(PromoteOutcome::Conflict { branch }) => Err(err(
            StatusCode::CONFLICT,
            &format!("merge conflict — resolve manually: git merge {branch}"),
        )),
        Err(NexusError::AgentNotFound(_)) => Err(err(StatusCode::NOT_FOUND, "agent not found")),
        Err(e) => Err(err(StatusCode::BAD_REQUEST, &e.to_string())),
    }
}
```

- [ ] **Step 4: Test** — append to `tests/api.rs`:
```rust
#[tokio::test]
async fn promote_non_isolated_agent_is_rejected() {
    use kaiju_core::agent::{Agent, AgentConfig, AgentType};
    let state = AppState::new();
    let agent = Agent::new(AgentConfig {
        agent_type: AgentType::Claude, model: None,
        workspace: std::path::PathBuf::from("/tmp"), prompt: None, extra_args: vec![],
    });
    let id = agent.id.clone();
    state.store.insert(agent); // isolate=false, no worktree
    let app = build_app(state);
    let resp = app
        .oneshot(json_request("POST", &format!("/agents/{id}/promote"), serde_json::json!({})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
```

- [ ] **Step 5: Verify** — `cargo test -p kaiju-daemon`; `cargo clippy -p kaiju-daemon` (clean); `cargo build`.

- [ ] **Step 6: Commit**
```bash
git add crates/kaiju-daemon/src/worktree.rs crates/kaiju-daemon/src/server.rs crates/kaiju-daemon/src/api.rs crates/kaiju-daemon/tests/api.rs
git commit -m "feat(daemon): POST /agents/:id/promote — merge a run's branch into the workspace"
```

---

### Task 2: Promote button in the comparison view

**Files:** `crates/kaiju-daemon/assets/dashboard.js`

- [ ] **Step 1: Add a Promote button per column.** In `renderComparison`, in the column header build (where the "Open" button is), add a Promote button beside it:
```javascript
      '<button onclick="select(\'' + a.id + '\')">Open</button>' +
      '<button onclick="promoteRun(\'' + a.id + '\')">Promote</button></div>' +
```
(Append the Promote button inside the same `.cmp-col-head`, before the closing `</div>`.)

- [ ] **Step 2: Add the `promoteRun` handler** (append to dashboard.js):
```javascript
async function promoteRun(id) {
  if (!confirm("Promote this run? It commits the run's changes and merges its branch into your workspace.")) return;
  try {
    const res = await api("/agents/" + id + "/promote", { method: "POST" });
    const d = await res.json();
    if (res.ok) { alert("Promoted — merged branch " + d.branch + " into your workspace."); }
    else { alert(d.error || "Promote failed."); }
  } catch (e) { alert("Promote failed."); }
}
```

- [ ] **Step 3: Verify** — `node --check crates/kaiju-daemon/assets/dashboard.js`; `node --test crates/kaiju-daemon/assets/dashboard-utils.test.js`; `cargo build -p kaiju-daemon`; `cargo test -p kaiju-daemon`.

- [ ] **Step 4: Commit**
```bash
git add crates/kaiju-daemon/assets/dashboard.js
git commit -m "feat(dashboard): Promote button on comparison runs"
```

---

### Task 3: Docs

**Files:** `README.md`

- [ ] **Step 1:** Extend the compare paragraph:
```markdown
**Promote** a run from the comparison view to commit its changes and merge its
branch into your workspace (refuses if your workspace is dirty; aborts cleanly on
conflict and tells you the branch to merge by hand).
```
Add the API row:
```markdown
| POST | `/agents/:id/promote` | Commit a run's changes and merge its branch into the workspace. |
```

- [ ] **Step 2: Commit**
```bash
git add README.md
git commit -m "docs: promote a winner"
```

---

## Self-Review Notes
- **Spec coverage:** git helpers + orchestrator + endpoint + safety mapping (T1), UI button (T2), docs (T3).
- **Safety:** workspace-dirty check happens BEFORE any commit/merge (non-destructive); conflict aborts the merge; only isolated runs with a live worktree are promotable.
- **Type consistency:** `MergeOutcome` (worktree) → `PromoteOutcome` (server) → status mapping (api). The handler's JSON `{status, branch}` / `{error}` matches what `promoteRun` reads.
- **No-tmux/no-git-state tests:** the non-isolated rejection is unit-testable; the merge happy-path shells out to git and is validated manually (consistent with worktree.rs's documented approach).
