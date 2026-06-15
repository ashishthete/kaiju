# Auto-Approve (unattended runs) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`).

**Goal:** An opt-in "auto-approve" toggle so an agent (especially bake-off runs) launches with its CLI's hands-off flag and finishes unattended — then you judge.

**Architecture:** Each adapter owns its auto-approve flag via a new `Adapter::auto_approve_args()`. When the toggle is set on create/compare, the daemon prepends those args to the agent's `extra_args` (which `build_command` already appends) — so no `AgentConfig`/`Agent` struct change. Per-CLI flags (confirmed): claude `--dangerously-skip-permissions`, codex `--full-auto`, gemini `--yolo`. Safe-ish because runs are in isolated git worktrees and it's opt-in.

---

### Task 1: `Adapter::auto_approve_args()` + per-CLI impls

**Files:** `crates/kaiju-core/src/adapter.rs`, `crates/kaiju-adapters/src/{claude,codex,gemini}.rs`

- [ ] **Step 1: Trait default** — in `adapter.rs`, in `pub trait Adapter`, add (near `resume_command`):
```rust
    /// Flags that make this CLI run unattended (no approval prompts), for
    /// auto-approve / bake-off runs. Default: none (the CLI has no such mode).
    fn auto_approve_args(&self) -> Vec<String> {
        Vec::new()
    }
```

- [ ] **Step 2: Per-adapter impls.**
  - `claude.rs` (`impl Adapter for ClaudeAdapter`):
    ```rust
    fn auto_approve_args(&self) -> Vec<String> {
        vec!["--dangerously-skip-permissions".to_string()]
    }
    ```
  - `codex.rs`:
    ```rust
    fn auto_approve_args(&self) -> Vec<String> {
        vec!["--full-auto".to_string()]
    }
    ```
  - `gemini.rs`:
    ```rust
    fn auto_approve_args(&self) -> Vec<String> {
        vec!["--yolo".to_string()]
    }
    ```

- [ ] **Step 3: Tests** — add one unit test per adapter file asserting the flag, e.g. in claude.rs `mod tests`:
```rust
    #[test]
    fn auto_approve_uses_skip_permissions() {
        assert_eq!(ClaudeAdapter.auto_approve_args(), vec!["--dangerously-skip-permissions".to_string()]);
    }
```
(codex → `--full-auto`, gemini → `--yolo`.) And in `adapter.rs` tests, the `Bare` adapter's default `auto_approve_args()` is empty:
```rust
    #[test]
    fn default_auto_approve_args_empty() {
        struct Bare;
        impl Adapter for Bare {
            fn agent_type(&self) -> AgentType { AgentType::Claude }
            fn build_command(&self, _c: &AgentConfig) -> String { String::new() }
            fn parse_output(&self, _o: &str) -> ParsedOutput { ParsedOutput::default() }
            fn display_name(&self) -> &str { "bare" }
        }
        assert!(Bare.auto_approve_args().is_empty());
    }
```

- [ ] **Step 4: Verify** — `cargo test -p kaiju-core -p kaiju-adapters` → pass; `cargo build` clean (dead_code on `auto_approve_args` until Task 2 — expected).

- [ ] **Step 5: Commit**
```bash
git add crates/kaiju-core/src/adapter.rs crates/kaiju-adapters/src/claude.rs crates/kaiju-adapters/src/codex.rs crates/kaiju-adapters/src/gemini.rs
git commit -m "feat(adapters): per-CLI auto_approve_args (unattended flags)"
```

---

### Task 2: wire auto-approve into create + compare

**Files:** `crates/kaiju-daemon/src/api.rs`, `crates/kaiju-daemon/src/server.rs`, `crates/kaiju-daemon/tests/api.rs`

- [ ] **Step 1: `CreateAgentRequest` + `CompareRequest` gain `auto_approve`.** Add to each (with `#[serde(default)]`):
```rust
    #[serde(default)]
    pub auto_approve: bool,
```

- [ ] **Step 2: Inject in `create_agent` (api.rs).** After resolving `agent_type` and BEFORE building the config's `extra_args`, prepend the adapter's auto-approve flags when requested:
```rust
    let mut extra_args = req.extra_args;
    if req.auto_approve {
        if let Some(adapter) = state.adapters.get(&agent_type) {
            let mut flags = adapter.auto_approve_args();
            flags.extend(extra_args);
            extra_args = flags;
        }
    }
```
Then use `extra_args` where the config is built (replace `extra_args: req.extra_args` with `extra_args`). (Confirm the exact current construction and adapt; `defaults.apply` still runs on the resulting config.)

- [ ] **Step 3: Inject in `spawn_compare_group` (server.rs).** It takes `agent_types` + builds a config per type with `extra_args: vec![]`. Add an `auto_approve: bool` parameter to `spawn_compare_group`, and per type:
```rust
    let extra_args = if auto_approve { adapter_lookup.auto_approve_args() } else { vec![] };
```
where `adapter_lookup = state.adapters.get(&agent_type)`. Use that as the config's `extra_args`. Thread `auto_approve` from the `compare` handler (`req.auto_approve`) into the call.

- [ ] **Step 4: Tests** — append to `tests/api.rs` (assert the flag lands in the stored agent's `extra_args`; `auto_start:false` so no tmux spawn):
```rust
#[tokio::test]
async fn create_with_auto_approve_injects_flag() {
    let state = AppState::new();
    let app = build_app(state.clone());
    let resp = app
        .oneshot(json_request("POST", "/agents", serde_json::json!({
            "agent_type": "claude", "workspace": "/tmp/x", "auto_start": false, "auto_approve": true
        })))
        .await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let agent = state.store.list().into_iter().next().unwrap();
    assert!(agent.extra_args.iter().any(|a| a == "--dangerously-skip-permissions"));
}

#[tokio::test]
async fn create_without_auto_approve_has_no_flag() {
    let state = AppState::new();
    let app = build_app(state.clone());
    let _ = app.oneshot(json_request("POST", "/agents", serde_json::json!({
        "agent_type": "claude", "workspace": "/tmp/x", "auto_start": false
    }))).await.unwrap();
    let agent = state.store.list().into_iter().next().unwrap();
    assert!(!agent.extra_args.iter().any(|a| a == "--dangerously-skip-permissions"));
}
```

- [ ] **Step 5: Verify** — `cargo test -p kaiju-daemon` (all pass incl. new); `cargo clippy -p kaiju-daemon` clean; `cargo build`.

- [ ] **Step 6: Commit**
```bash
git add crates/kaiju-daemon/src/api.rs crates/kaiju-daemon/src/server.rs crates/kaiju-daemon/tests/api.rs
git commit -m "feat(daemon): auto_approve on create + compare (inject CLI unattended flags)"
```

---

### Task 3: UI toggles

**Files:** `crates/kaiju-daemon/src/dashboard.rs`, `crates/kaiju-daemon/assets/dashboard.js`

- [ ] **Step 1: New-agent modal checkbox.** In `#newmodal`, inside the Advanced `<details>` (next to the isolate checkbox), add:
```html
        <label class="check"><input type="checkbox" id="n-auto"> Auto-approve (run unattended — skips approval prompts)</label>
```
In `createAgent()` (dashboard.js), include it in the POST body: read `document.getElementById("n-auto").checked` and add `auto_approve: <that>` to the JSON. (Read the current `createAgent` to match its body-building.)

- [ ] **Step 2: Compare modal checkbox.** In `#comparemodal`, after the CLI checkboxes / before the note, add:
```html
    <label class="check"><input type="checkbox" id="cmp-auto" checked> Auto-approve runs (finish unattended, then judge)</label>
```
In `submitCompare()`, add `auto_approve: document.getElementById("cmp-auto").checked` to the POST body. (Default checked, since the whole point of a bake-off is hands-off completion.)

- [ ] **Step 3: Verify** — `node --check dashboard.js`; `node --test dashboard-utils.test.js`; `cargo build -p kaiju-daemon`; `cargo test -p kaiju-daemon` (page-references-scripts test stays green; don't break the raw string).

- [ ] **Step 4: Commit**
```bash
git add crates/kaiju-daemon/src/dashboard.rs crates/kaiju-daemon/assets/dashboard.js
git commit -m "feat(dashboard): auto-approve toggles on New agent + Compare"
```

---

### Task 4: Docs

**Files:** `README.md`

- [ ] **Step 1:** Add to the bake-off / agent docs:
```markdown
- **Auto-approve.** Tick *auto-approve* (default on for bake-offs) and each run
  launches with its CLI's unattended flag (`claude --dangerously-skip-permissions`,
  `codex --full-auto`, `gemini --yolo`) so it finishes without prompting — runs
  are isolated in their own worktree. Opt-in; off by default for single agents.
```

- [ ] **Step 2: Commit**
```bash
git add README.md
git commit -m "docs: auto-approve"
```

---

## Self-Review Notes
- **No struct churn:** auto-approve rides existing `extra_args`; the adapter owns the flag via `auto_approve_args()`. `AgentConfig`/`Agent` unchanged.
- **Opt-in + isolated:** off by default for single agents; the flags only skip prompts, and runs live in their own worktree.
- **Type consistency:** `auto_approve_args()` (adapter) → injected into `extra_args` by `create_agent`/`spawn_compare_group` → appended by `build_command`. `auto_approve` flows request → daemon. UI checkbox ids `#n-auto`/`#cmp-auto` → POST body.
- **Tests:** per-adapter flag (unit) + the flag lands in the stored agent (integration, no tmux).
