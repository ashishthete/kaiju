# Adopt a Session (Resume by ID) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a user adopt an existing Claude Code conversation into Kaiju by resuming it (`claude --resume <id>`) inside a managed tmux session, picked from a list of resumable sessions for a workspace.

**Architecture:** A normal Kaiju agent whose launch command is a resume-by-id. Two new adapter methods carry the CLI-specific knowledge (`list_sessions`, `resume_session_command`); the daemon adds `GET /sessions` (discovery) and `POST /agents/adopt` (create+spawn), and the dashboard adds an Adopt modal. Claude-only in v1 — other adapters return empty/None and simply don't appear.

**Tech Stack:** Rust (kaiju-core trait + kaiju-adapters Claude impl + kaiju-daemon axum endpoints), vanilla-JS dashboard.

**Known v1 limitation (documented, not fixed here):** token/cost metrics won't attribute for adopted agents — `claude_transcript::find_transcript` matches sessions that *started at/after* the agent's start time, but a resumed transcript began earlier. The terminal, status, and control all work; only the metrics columns stay blank. Fixing it (match by stored session id) is a noted follow-up.

---

## File Structure

- `crates/kaiju-core/src/adapter.rs` — **modify**: add `SessionInfo` struct + two default trait methods.
- `crates/kaiju-adapters/src/claude_transcript.rs` — **modify**: `pub(crate)` discovery helper `list_workspace_sessions`.
- `crates/kaiju-adapters/src/claude.rs` — **modify**: implement `list_sessions` + `resume_session_command`.
- `crates/kaiju-daemon/src/server.rs` — **modify**: `adopt_agent_internal` helper.
- `crates/kaiju-daemon/src/api.rs` — **modify**: `GET /sessions`, `POST /agents/adopt` routes + handlers + request types.
- `crates/kaiju-daemon/src/dashboard.rs` — **modify**: Adopt button + modal HTML/CSS.
- `crates/kaiju-daemon/assets/dashboard.js` — **modify**: adopt flow.
- Tests alongside each.

---

### Task 1: `SessionInfo` + adapter trait methods

**Files:**
- Modify: `crates/kaiju-core/src/adapter.rs`

- [ ] **Step 1: Write the failing test**

In `crates/kaiju-core/src/adapter.rs`, add to the `#[cfg(test)] mod tests` block a test that a trivial adapter gets the defaults. First add a tiny test adapter or reuse an existing pattern — use this self-contained test:

```rust
    #[test]
    fn adapter_defaults_for_sessions_are_empty() {
        struct Bare;
        impl Adapter for Bare {
            fn agent_type(&self) -> AgentType { AgentType::Claude }
            fn build_command(&self, _c: &AgentConfig) -> String { String::new() }
            fn parse_output(&self, _o: &str) -> ParsedOutput { ParsedOutput::default() }
            fn display_name(&self) -> &str { "bare" }
        }
        let a = Bare;
        let ws = std::path::Path::new("/tmp/x");
        assert!(a.list_sessions(ws).is_empty());
        let cfg = AgentConfig {
            agent_type: AgentType::Claude, model: None,
            workspace: ws.to_path_buf(), prompt: None, extra_args: vec![],
        };
        assert!(a.resume_session_command(&cfg, "abc").is_none());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kaiju-core adapter::tests::adapter_defaults_for_sessions_are_empty`
Expected: FAIL — `SessionInfo`/`list_sessions`/`resume_session_command` not defined.

- [ ] **Step 3: Add the type and trait methods**

In `crates/kaiju-core/src/adapter.rs`, add near the top (after imports, before the trait). Confirm `serde::{Serialize, Deserialize}` is available in this crate (it's used elsewhere); import if needed:

```rust
use serde::{Deserialize, Serialize};

/// A resumable CLI session discovered for a workspace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionInfo {
    /// CLI session id (used to resume).
    pub id: String,
    /// Last-active time, Unix seconds (for sorting + "2h ago" display).
    pub last_active_unix: i64,
    /// First user prompt, truncated — a human-readable label.
    pub first_prompt: String,
}
```

Add these two methods inside `pub trait Adapter` (next to `resume_command`):

```rust
    /// Resumable sessions this CLI has recorded for `workspace`, newest first.
    /// Default: none (the CLI's session storage is unknown to Kaiju).
    fn list_sessions(&self, _workspace: &std::path::Path) -> Vec<SessionInfo> {
        Vec::new()
    }

    /// Command to resume a *specific* session by id (e.g. `claude --resume <id>`),
    /// for adopting an existing conversation. Default: unsupported.
    fn resume_session_command(
        &self,
        _config: &AgentConfig,
        _session_id: &str,
    ) -> Option<String> {
        None
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p kaiju-core adapter::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/kaiju-core/src/adapter.rs
git commit -m "feat(core): SessionInfo + adapter session-discovery/resume-by-id methods"
```

---

### Task 2: Claude session discovery helper

**Files:**
- Modify: `crates/kaiju-adapters/src/claude_transcript.rs`

- [ ] **Step 1: Write the failing test**

Append to the `#[cfg(test)] mod tests` in `claude_transcript.rs`. It uses the existing `KAIJU_CLAUDE_PROJECTS` override to point at a temp dir:

```rust
    #[test]
    fn list_workspace_sessions_reads_id_and_first_prompt() {
        use std::io::Write;
        let tmp = std::env::temp_dir().join("kaiju-sessions-test-list");
        let ws = std::path::Path::new("/Users/x/repo");
        let slug = project_slug(ws);
        let dir = tmp.join(&slug);
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&dir).unwrap();
        // A session transcript whose first line is a user message.
        let mut f = std::fs::File::create(dir.join("sess-1.jsonl")).unwrap();
        writeln!(f, r#"{{"type":"user","message":{{"role":"user","content":"refactor the parser"}}}}"#).unwrap();
        writeln!(f, r#"{{"type":"assistant","message":{{"id":"m1","usage":{{"output_tokens":5}}}}}}"#).unwrap();

        std::env::set_var("KAIJU_CLAUDE_PROJECTS", &tmp);
        let sessions = list_workspace_sessions(ws);
        std::env::remove_var("KAIJU_CLAUDE_PROJECTS");

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "sess-1");
        assert_eq!(sessions[0].first_prompt, "refactor the parser");
        assert!(sessions[0].last_active_unix > 0);
    }

    #[test]
    fn list_workspace_sessions_empty_when_no_dir() {
        std::env::set_var("KAIJU_CLAUDE_PROJECTS", std::env::temp_dir().join("kaiju-nope-xyz"));
        let out = list_workspace_sessions(std::path::Path::new("/no/such/ws"));
        std::env::remove_var("KAIJU_CLAUDE_PROJECTS");
        assert!(out.is_empty());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kaiju-adapters claude_transcript::tests::list_workspace_sessions`
Expected: FAIL — `list_workspace_sessions` not defined.

- [ ] **Step 3: Implement the helper**

In `claude_transcript.rs`, make `projects_root` visible to the crate (change `fn projects_root()` to `pub(crate) fn projects_root()`). Add `use kaiju_core::adapter::SessionInfo;` at the top. Then add:

```rust
/// First user-message text in a transcript, truncated for display. Reads only
/// the head of the file. Handles `content` as a plain string or an array of
/// content blocks (uses the first text block).
fn first_user_prompt(path: &Path) -> String {
    let Ok(file) = std::fs::File::open(path) else {
        return String::new();
    };
    for line in BufReader::new(file).lines().take(50).map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        if v.get("type").and_then(Value::as_str) != Some("user") {
            continue;
        }
        let content = match v.get("message").and_then(|m| m.get("content")) {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Array(items)) => items
                .iter()
                .find_map(|it| it.get("text").and_then(Value::as_str))
                .unwrap_or("")
                .to_string(),
            _ => String::new(),
        };
        let trimmed = content.trim();
        if trimmed.is_empty() {
            continue;
        }
        return trimmed.chars().take(80).collect();
    }
    String::new()
}

/// Last-modified time of a file as Unix seconds (0 if unavailable).
fn modified_unix(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Resumable Claude sessions recorded for `workspace`, newest first.
/// Best-effort: a missing/unreadable projects dir yields an empty list.
pub(crate) fn list_workspace_sessions(workspace: &Path) -> Vec<SessionInfo> {
    let Some(dir) = projects_root().map(|r| r.join(project_slug(workspace))) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut sessions: Vec<SessionInfo> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("jsonl"))
        .filter_map(|p| {
            let id = p.file_stem()?.to_str()?.to_string();
            Some(SessionInfo {
                id,
                last_active_unix: modified_unix(&p),
                first_prompt: first_user_prompt(&p),
            })
        })
        .collect();
    sessions.sort_by(|a, b| b.last_active_unix.cmp(&a.last_active_unix));
    sessions
}
```

(Confirm `use serde_json::Value;`, `use std::io::{BufRead, BufReader};`, and `use std::path::{Path, PathBuf};` are already present at the top — they are, since `aggregate_usage`/`session_start_unix` use them.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p kaiju-adapters claude_transcript::`
Expected: PASS (the two new tests + existing ones).

- [ ] **Step 5: Commit**

```bash
git add crates/kaiju-adapters/src/claude_transcript.rs
git commit -m "feat(adapters): discover resumable Claude sessions for a workspace"
```

---

### Task 3: Claude adapter — `list_sessions` + `resume_session_command`

**Files:**
- Modify: `crates/kaiju-adapters/src/claude.rs`

- [ ] **Step 1: Write the failing test**

Append to `#[cfg(test)] mod tests` in `claude.rs`:

```rust
    #[test]
    fn resume_session_command_uses_resume_flag_and_id() {
        let cfg = AgentConfig {
            agent_type: AgentType::Claude,
            model: Some("claude-opus-4-8".to_string()),
            workspace: std::path::PathBuf::from("/tmp/repo"),
            prompt: None,
            extra_args: vec![],
        };
        let cmd = ClaudeAdapter.resume_session_command(&cfg, "abc123").unwrap();
        assert!(cmd.contains("cd /tmp/repo &&"));
        assert!(cmd.contains("--resume abc123"));
        assert!(cmd.contains("--model claude-opus-4-8"));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kaiju-adapters claude::tests::resume_session_command`
Expected: FAIL — method not implemented (uses the trait default returning `None` → `unwrap` panics).

- [ ] **Step 3: Implement both methods on `ClaudeAdapter`**

Add `use kaiju_core::adapter::SessionInfo;` to the imports in `claude.rs`. Inside `impl Adapter for ClaudeAdapter`, add:

```rust
    fn list_sessions(&self, workspace: &std::path::Path) -> Vec<SessionInfo> {
        crate::claude_transcript::list_workspace_sessions(workspace)
    }

    fn resume_session_command(
        &self,
        config: &AgentConfig,
        session_id: &str,
    ) -> Option<String> {
        let bin = crate::binary::agent_binary("KAIJU_CLAUDE_BIN", "claude");
        let mut cmd = format!(
            "cd {} && {bin} --resume {session_id}",
            config.workspace.display()
        );
        if let Some(model) = config.model.as_deref().or(self.default_model()) {
            cmd.push_str(&format!(" --model {model}"));
        }
        for arg in &config.extra_args {
            cmd.push_str(&format!(" {arg}"));
        }
        Some(cmd)
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p kaiju-adapters claude::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/kaiju-adapters/src/claude.rs
git commit -m "feat(adapters): Claude list_sessions + resume-by-id command"
```

---

### Task 4: Daemon endpoints — `GET /sessions` and `POST /agents/adopt`

**Files:**
- Modify: `crates/kaiju-daemon/src/server.rs` (add `adopt_agent_internal`)
- Modify: `crates/kaiju-daemon/src/api.rs` (routes, handlers, request types)
- Test: `crates/kaiju-daemon/tests/api.rs`

- [ ] **Step 1: Write the failing integration tests**

Append to `crates/kaiju-daemon/tests/api.rs`:

```rust
#[tokio::test]
async fn list_sessions_unsupported_type_returns_empty() {
    let app = build_app(AppState::new());
    // "gemini" has no session discovery → empty list, 200.
    let resp = app
        .oneshot(get_request("/sessions?workspace=/tmp/x&type=gemini"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn adopt_rejects_blank_session_id() {
    let app = build_app(AppState::new());
    let resp = app
        .oneshot(json_request(
            "POST",
            "/agents/adopt",
            serde_json::json!({ "agent_type": "claude", "workspace": "/tmp/x", "session_id": "" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p kaiju-daemon --test api list_sessions_unsupported_type_returns_empty adopt_rejects_blank_session_id`
Expected: FAIL — routes return 404.

- [ ] **Step 3: Add `adopt_agent_internal` to `server.rs`**

After `resume_agent_internal` in `crates/kaiju-daemon/src/server.rs`, add:

```rust
/// Create an agent that resumes an existing CLI session by id, spawn it, and
/// return its id. Like start, but the launch command is the adapter's
/// resume-by-id command. Never creates a worktree (resume runs in place).
pub fn adopt_agent_internal(
    state: &AppState,
    config: &kaiju_core::agent::AgentConfig,
    session_id: &str,
) -> Result<String> {
    let adapter = state
        .adapters
        .get(&config.agent_type)
        .ok_or_else(|| NexusError::Adapter(format!("no adapter for {}", config.agent_type)))?;

    let command = adapter
        .resume_session_command(config, session_id)
        .ok_or_else(|| {
            NexusError::Adapter(format!("{} does not support resume", config.agent_type))
        })?;

    let agent = kaiju_core::agent::Agent::new(config.clone());
    let id = agent.id.clone();
    let session_name = agent.session_name.clone();
    state.store.insert(agent);

    TmuxManager::create_session(
        &session_name,
        &config.workspace.display().to_string(),
        &command,
    )?;
    state.store.mark_started(&id, chrono::Utc::now());
    Ok(id)
}
```

- [ ] **Step 4: Add request types, routes, and handlers to `api.rs`**

In `crates/kaiju-daemon/src/api.rs`, register routes inside `routes()` (after the `/agents/adopt`-adjacent area, e.g. right after the `/agents/:id/...` block or near `/tasks`):

```rust
        .route("/sessions", get(list_sessions))
        .route("/agents/adopt", post(adopt_agent))
```

Add request type (near the other `#[derive(Deserialize)]` request structs):

```rust
#[derive(Deserialize)]
pub struct AdoptRequest {
    pub agent_type: String,
    pub workspace: String,
    pub session_id: String,
    pub model: Option<String>,
}

#[derive(Deserialize)]
pub struct SessionsQuery {
    pub workspace: String,
    #[serde(rename = "type")]
    pub agent_type: String,
}
```

Add handlers (near the other handlers):

```rust
/// `GET /sessions?workspace=<path>&type=<agent_type>` — resumable CLI sessions
/// the adapter can discover for that workspace (empty if it can't).
async fn list_sessions(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<SessionsQuery>,
) -> impl IntoResponse {
    let agent_type: AgentType = match q.agent_type.parse() {
        Ok(t) => t,
        Err(_) => return Json(Vec::<kaiju_core::adapter::SessionInfo>::new()),
    };
    let sessions = match state.adapters.get(&agent_type) {
        Some(adapter) => adapter.list_sessions(std::path::Path::new(&q.workspace)),
        None => Vec::new(),
    };
    Json(sessions)
}

/// `POST /agents/adopt` — create an agent that resumes an existing session by id.
async fn adopt_agent(
    State(state): State<AppState>,
    Json(req): Json<AdoptRequest>,
) -> impl IntoResponse {
    if req.agent_type.trim().is_empty()
        || req.workspace.trim().is_empty()
        || req.session_id.trim().is_empty()
    {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "agent_type, workspace, and session_id are required",
        ));
    }
    let agent_type: AgentType = req.agent_type.parse().expect("infallible");
    let config = AgentConfig {
        agent_type,
        model: req.model,
        workspace: PathBuf::from(&req.workspace),
        prompt: None,
        extra_args: vec![],
    };
    match crate::server::adopt_agent_internal(&state, &config, &req.session_id) {
        Ok(id) => {
            let agent = state.store.get(&id).unwrap();
            Ok((StatusCode::CREATED, Json(AgentResponse::from(&agent))))
        }
        Err(e) => {
            let code = match e {
                NexusError::Adapter(_) => StatusCode::BAD_REQUEST,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            Err(err(code, &e.to_string()))
        }
    }
}
```

Add `use kaiju_core::NexusError;` to `api.rs` imports if not present (check the top of the file; other handlers reference it via `kaiju_core::NexusError` in `resume_agent` — match that style: it uses `use kaiju_core::NexusError;` locally in `resume_agent`. Use a local `use` inside `adopt_agent` for consistency, or a top-level import — match the file).

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p kaiju-daemon --test api list_sessions_unsupported_type_returns_empty adopt_rejects_blank_session_id`
Expected: PASS. Then run the whole suite: `cargo test -p kaiju-daemon` → all pass.

- [ ] **Step 6: Commit**

```bash
git add crates/kaiju-daemon/src/server.rs crates/kaiju-daemon/src/api.rs crates/kaiju-daemon/tests/api.rs
git commit -m "feat(daemon): /sessions discovery and /agents/adopt endpoints"
```

---

### Task 5: Dashboard — Adopt button + modal

**Files:**
- Modify: `crates/kaiju-daemon/src/dashboard.rs` (button, modal, CSS)
- Modify: `crates/kaiju-daemon/assets/dashboard.js` (adopt flow)

- [ ] **Step 1: Add the Adopt button**

In `crates/kaiju-daemon/src/dashboard.rs`, find the toolbar/header where the "New agent" button lives (search for the New-agent trigger, e.g. `openNew()` / a button near the top toolbar). Add an Adopt button beside it:

```html
    <button onclick="openAdopt()">Adopt session</button>
```

- [ ] **Step 2: Add the Adopt modal**

Near the existing `<dialog id="newmodal" ...>` block, add a sibling dialog:

```html
  <dialog id="adoptmodal" class="modal" onclick="if(event.target===this)closeAdopt()">
    <div class="modal-head">
      <h2>Adopt a session</h2>
      <button type="button" class="icon" onclick="closeAdopt()" title="Close">&times;</button>
    </div>
    <label class="field">
      <span>Workspace path <em>*</em></span>
      <input id="ad-ws" placeholder="/path/to/repo" autocomplete="off" onchange="loadSessions()">
    </label>
    <label class="field">
      <span>Agent</span>
      <select id="ad-type" onchange="loadSessions()">
        <option value="claude">claude</option>
        <option value="codex">codex</option>
        <option value="gemini">gemini</option>
      </select>
    </label>
    <div id="ad-sessions" class="device-list"></div>
    <div class="note">Close the original session first so two clients don't drive one conversation.</div>
    <div class="modal-actions">
      <button type="button" onclick="closeAdopt()">Cancel</button>
    </div>
  </dialog>
```

(Reuses the existing `.modal`, `.field`, `.modal-actions`, `.note`, and `.device-list` classes.)

- [ ] **Step 3: Add the adopt JS**

Append to `crates/kaiju-daemon/assets/dashboard.js`:

```javascript
// --- Adopt an existing session ---

function openAdopt() {
  document.getElementById("ad-sessions").innerHTML = "";
  document.getElementById("adoptmodal").showModal();
}
function closeAdopt() { document.getElementById("adoptmodal").close(); }

async function loadSessions() {
  const ws = document.getElementById("ad-ws").value.trim();
  const type = document.getElementById("ad-type").value;
  const box = document.getElementById("ad-sessions");
  if (!ws) { box.innerHTML = ""; return; }
  box.innerHTML = '<div class="pop-hint">Loading…</div>';
  try {
    const res = await api("/sessions?workspace=" + encodeURIComponent(ws) + "&type=" + encodeURIComponent(type));
    const sessions = await res.json();
    if (!sessions.length) { box.innerHTML = '<div class="pop-hint">No resumable sessions found.</div>'; return; }
    box.innerHTML = sessions.map(function (s) {
      const when = timeAgo(new Date(s.last_active_unix * 1000).toISOString(), Date.now());
      const label = esc(s.first_prompt || "(no prompt)") + " · " + when + " · " + esc(s.id.slice(0, 8));
      return '<div class="device-row"><span>' + label +
        '</span><button onclick="adopt(\'' + encodeURIComponent(s.id) + '\')">Adopt</button></div>';
    }).join("");
  } catch (e) { box.innerHTML = '<div class="pop-hint">Could not load sessions.</div>'; }
}

async function adopt(encodedId) {
  const ws = document.getElementById("ad-ws").value.trim();
  const type = document.getElementById("ad-type").value;
  try {
    const res = await api("/agents/adopt", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ agent_type: type, workspace: ws, session_id: decodeURIComponent(encodedId) }),
    });
    if (!res.ok) { alert("Adopt failed."); return; }
    const agent = await res.json();
    closeAdopt();
    refresh();           // reload the fleet
    select(agent.id);    // open the new agent's detail panel
  } catch (e) { alert("Adopt failed."); }
}
```

(Confirm the function names `refresh`, `select`, `timeAgo`, `esc` exist in dashboard.js/dashboard-utils.js — they do: `timeAgo`/`esc` are in dashboard-utils.js, and the fleet reload + detail-open helpers exist in dashboard.js. If the actual names differ, adapt to the real helpers — e.g. the fleet reload may be `load()`/`poll()`; grep before finalizing.)

- [ ] **Step 4: Verify**

Run from the worktree:
- `node --check crates/kaiju-daemon/assets/dashboard.js` → clean
- `node --test crates/kaiju-daemon/assets/dashboard-utils.test.js` → still passes
- `cargo build -p kaiju-daemon` → clean
- `cargo test -p kaiju-daemon` → all pass (PAGE-references-scripts test still green)

- [ ] **Step 5: Commit**

```bash
git add crates/kaiju-daemon/src/dashboard.rs crates/kaiju-daemon/assets/dashboard.js
git commit -m "feat(dashboard): adopt-session modal (pick a resumable session)"
```

---

### Task 6: Docs

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Document adopt**

In `README.md`, near the agent-creation/usage docs, add a short subsection:

```markdown
**Adopt a session:** to bring an existing Claude Code conversation under Kaiju,
click **Adopt session**, enter the workspace, pick a resumable session, and
Adopt — Kaiju resumes it (`claude --resume <id>`) in a managed tmux session.
Close the original first so two clients don't drive one conversation. (Token
metrics don't attribute for adopted sessions yet.)
```

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs: adopt a session"
```

---

## Self-Review Notes

- **Spec coverage:** discovery (Tasks 2,3,4), resume-by-id (Tasks 1,3), endpoints (Task 4), UI (Task 5), Claude-only-via-defaults (Task 1), lifecycle = normal agent (Task 4 reuses store + create_session), metrics limitation documented (Task 6 + plan header).
- **Type consistency:** `SessionInfo {id, last_active_unix, first_prompt}` defined in Task 1 is used verbatim in Tasks 2/3/4 and the JS reads `last_active_unix`/`first_prompt`/`id`. `adopt_agent_internal(state, &AgentConfig, &str) -> Result<String>` (Task 4 server) matches its caller in the `adopt_agent` handler.
- **No tmux in tests:** adapter + discovery logic is unit-tested without tmux; the adopt happy-path (which shells to tmux) is covered only at the validation/command level, consistent with the existing suite.
