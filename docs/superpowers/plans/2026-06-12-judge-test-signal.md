# Judge Test Signal — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`).

**Goal:** Give the comparison judge an objective signal — run a user-supplied test command in each compare worktree and feed pass/fail + output to the judge alongside the diff, so the verdict reflects "which actually works", not just looks.

**Architecture:** Extend the existing `judge` module with a `run_tests` side-effect and a per-candidate test summary woven into `build_prompt`. `POST /compare/judge` accepts an optional `test_cmd`; when present it runs in each agent's worktree (`run_dir`) before judging. The UI adds an optional test-command input next to Judge. When `test_cmd` is absent, behavior is unchanged (diff-only).

**Security note:** `test_cmd` is a user-supplied shell command run server-side via `sh -c` in each worktree. This endpoint is behind auth (loopback/paired only), and the user supplies the command explicitly (like a CI command) — consistent with Kaiju already running CLIs/commands locally. Documented in the README.

---

### Task 1: `judge` module — test summary + `run_tests` + prompt

**Files:** `crates/kaiju-daemon/src/judge.rs`

- [ ] **Step 1: Extend `Candidate` + add `TestSummary`.** Add above `Candidate`:
```rust
/// Outcome of running the project's test command in a candidate's worktree.
pub struct TestSummary {
    pub passed: bool,
    /// Short human line (exit status + tail of output) for the judge prompt.
    pub summary: String,
}
```
Add a field to `Candidate` (after `diff`):
```rust
    /// Test outcome, when a test command was supplied. `None` = diff-only judging.
    pub test: Option<TestSummary>,
```

- [ ] **Step 2: Failing test** — append to `mod tests`:
```rust
    #[test]
    fn prompt_includes_test_outcome_when_present() {
        let cands = vec![Candidate {
            label: "A".into(), agent_type: "claude".into(), diff: "+x".into(),
            test: Some(TestSummary { passed: true, summary: "exit 0 (ok)".into() }),
        }];
        let p = build_prompt("t", &cands);
        assert!(p.contains("Tests: PASS"));
        assert!(p.contains("exit 0 (ok)"));
    }
```
Also update the two existing `Candidate { ... }` constructions in the other tests to add `test: None,` (so they compile).

- [ ] **Step 3: Weave the test line into `build_prompt`.** Update the rubric sentence and add the test block. Replace the rubric string's first sentence to mention tests, and inside the per-candidate loop, before pushing the diff, add the test line. Concretely, change the loop body to:
```rust
    for c in candidates {
        s.push_str(&format!("\n### Candidate {}\n", c.label));
        if let Some(t) = &c.test {
            let verdict = if t.passed { "PASS" } else { "FAIL" };
            s.push_str(&format!("Tests: {verdict} — {}\n", t.summary));
        }
        let truncated: String = c.diff.chars().take(DIFF_CAP_CHARS).collect();
        s.push_str(&truncated);
        if c.diff.chars().count() > DIFF_CAP_CHARS {
            s.push_str("\n…[truncated]");
        }
        s.push('\n');
    }
```
And update the opening rubric line to weigh tests, replacing the first sentence with:
```rust
    let mut s = String::from(
        "You are judging candidate solutions to a coding task. When test results \
         are given, weigh them heavily — a candidate whose tests pass beats one \
         that only looks cleaner. Rank them best to worst on test results, \
         correctness, completeness, and code quality. Give a one-line rationale \
         for each candidate, then name the winner. Be concise.\n\n## Task\n",
    );
```

- [ ] **Step 4: Add `run_tests`.** Add after `run_judge`:
```rust
/// Run `cmd` (via `sh -c`) in `workdir` with a timeout, capturing pass/fail and
/// a tail of the combined output for the judge. Best-effort: a spawn failure or
/// timeout is reported as a failing test with the reason.
pub async fn run_tests(workdir: &Path, cmd: &str) -> TestSummary {
    let fut = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(workdir)
        .output();
    match tokio::time::timeout(std::time::Duration::from_secs(300), fut).await {
        Ok(Ok(out)) => {
            let mut combined = String::from_utf8_lossy(&out.stdout).to_string();
            combined.push_str(&String::from_utf8_lossy(&out.stderr));
            let tail: String = {
                let chars: Vec<char> = combined.chars().collect();
                let start = chars.len().saturating_sub(1500);
                chars[start..].iter().collect()
            };
            TestSummary {
                passed: out.status.success(),
                summary: format!("exit {}\n{}", out.status.code().unwrap_or(-1), tail.trim()),
            }
        }
        Ok(Err(e)) => TestSummary { passed: false, summary: format!("could not run tests: {e}") },
        Err(_) => TestSummary { passed: false, summary: "tests timed out (300s)".to_string() },
    }
}
```

- [ ] **Step 5: Verify** — `cargo test -p kaiju-daemon judge::` → all pass (incl. new + updated). `cargo build -p kaiju-daemon` → clean (a `dead_code` warning for `run_tests`/`TestSummary` is expected until Task 2). `cargo clippy -p kaiju-daemon` (run after Task 2 for clean).

- [ ] **Step 6: Commit**
```bash
git add crates/kaiju-daemon/src/judge.rs
git commit -m "feat(daemon): judge test signal — run_tests + test-aware prompt"
```

---

### Task 2: `/compare/judge` runs the test command per worktree

**Files:** `crates/kaiju-daemon/src/api.rs`, `crates/kaiju-daemon/tests/api.rs`

- [ ] **Step 1: Extend `JudgeRequest`** with an optional command:
```rust
#[derive(Deserialize)]
pub struct JudgeRequest {
    pub group_id: String,
    #[serde(default)]
    pub test_cmd: Option<String>,
}
```

- [ ] **Step 2: Run tests per candidate in `compare_judge`.** Where the handler builds `candidates`, change it from a `.map()` to an async loop so it can `await` `run_tests`. Replace the `let candidates = agents.iter().enumerate().map(...).collect();` block with:
```rust
    let test_cmd = req.test_cmd.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let mut candidates: Vec<crate::judge::Candidate> = Vec::new();
    for (i, a) in agents.iter().enumerate() {
        let diff = crate::worktree::WorktreeManager::diff(a.run_dir()).unwrap_or_default();
        let test = match test_cmd {
            Some(cmd) => Some(crate::judge::run_tests(a.run_dir(), cmd).await),
            None => None,
        };
        candidates.push(crate::judge::Candidate {
            label: crate::judge::label_for(i),
            agent_type: a.agent_type.to_string(),
            diff: if diff.trim().is_empty() { "(no changes)".to_string() } else { diff },
            test,
        });
    }
```
(The `legend`, `task`, `workspace`, and the `run_judge` call stay as they are. Confirm the existing test `judge_rejects_group_with_fewer_than_two_runs` still compiles — it sends no `test_cmd`, fine via `#[serde(default)]`.)

- [ ] **Step 3: Verify** — `cargo test -p kaiju-daemon` → all pass. `cargo clippy -p kaiju-daemon` → clean (the Task 1 dead_code is now consumed). `cargo build` → clean.

- [ ] **Step 4: Commit**
```bash
git add crates/kaiju-daemon/src/api.rs crates/kaiju-daemon/tests/api.rs
git commit -m "feat(daemon): /compare/judge runs an optional test command per worktree"
```

---

### Task 3: UI — optional test-command input

**Files:** `crates/kaiju-daemon/src/dashboard.rs`, `crates/kaiju-daemon/assets/dashboard.js`

- [ ] **Step 1: Add the input** in the `#compare-panel` `.detail-head`, before the Judge button:
```html
      <input id="cmp-test-cmd" placeholder="test cmd (optional, e.g. cargo test)" title="Run this in each worktree and feed pass/fail to the judge" style="flex:0 1 18rem">
      <button id="cmp-judge-btn" onclick="judgeComparison()">Judge</button>
```

- [ ] **Step 2: JS — send the command + remember it.** Update `judgeComparison()` to read the input (defaulting from localStorage), persist it, and include it in the POST body. Change the body line and add the read/persist at the top of the function:
```javascript
async function judgeComparison() {
  if (!compareGroup) return;
  const btn = document.getElementById("cmp-judge-btn");
  const box = document.getElementById("cmp-verdict");
  const testCmd = document.getElementById("cmp-test-cmd").value.trim();
  localStorage.setItem("kaiju_test_cmd", testCmd);
  btn.disabled = true;
  box.hidden = false;
  box.innerHTML = '<span class="spinner"></span> ' + (testCmd ? "Running tests + judging…" : "Judging…");
  try {
    const res = await api("/compare/judge", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ group_id: compareGroup, test_cmd: testCmd || null }),
    });
    if (!res.ok) { box.textContent = (await res.json()).error || "Judge failed."; btn.disabled = false; return; }
    const d = await res.json();
    const legend = (d.legend || []).map(function (l) { return esc(l.label) + " = " + esc(l.agent_type); }).join("  \xb7  ");
    box.innerHTML = '<div class="cmp-legend">' + legend + '</div>' +
      '<pre class="cmp-verdict-text">' + esc(d.verdict) + '</pre>';
  } catch (e) { box.textContent = "Judge failed."; }
  btn.disabled = false;
}
```
And in `openComparison(groupId)`, prefill the input from localStorage (after the panel is shown):
```javascript
  document.getElementById("cmp-test-cmd").value = localStorage.getItem("kaiju_test_cmd") || "";
```

- [ ] **Step 3: Verify** — `node --check dashboard.js`; `node --test dashboard-utils.test.js`; `cargo build -p kaiju-daemon`; `cargo test -p kaiju-daemon`.

- [ ] **Step 4: Commit**
```bash
git add crates/kaiju-daemon/src/dashboard.rs crates/kaiju-daemon/assets/dashboard.js
git commit -m "feat(dashboard): optional test command for the judge"
```

---

### Task 4: Docs

**Files:** `README.md`

- [ ] **Step 1:** Extend the judge sentence:
```markdown
Provide an optional **test command** (e.g. `cargo test`) and the judge runs it in
each worktree, weighing pass/fail over looks. The command runs locally via `sh -c`
in each run's worktree (auth-gated).
```
Update the `/compare/judge` API row to mention `test_cmd`:
```markdown
| POST | `/compare/judge` | LLM-rank a compare group's runs (anonymized); optional `test_cmd` run per worktree for an objective signal. |
```

- [ ] **Step 2: Commit**
```bash
git add README.md
git commit -m "docs: judge test signal"
```

---

## Self-Review Notes
- **Spec coverage:** test summary + run_tests + prompt (T1), endpoint runs per-worktree (T2), UI input + remembered cmd (T3), docs incl. the `sh -c` security note (T4).
- **Backward compatible:** `test_cmd` is optional (`#[serde(default)]`); absent → diff-only judging, unchanged. The existing judge tests pass without it.
- **Type consistency:** `TestSummary {passed, summary}`, `Candidate.test: Option<TestSummary>`, `run_tests(&Path, &str) -> TestSummary` match the handler and `build_prompt`.
- **Anonymization preserved:** the test line uses PASS/FAIL + output, never the CLI name; the existing anonymization test still holds.
- **Latency:** tests run sequentially per candidate (≤3) with a 300s cap each; the Judge call can take minutes — the UI shows a "Running tests + judging…" spinner and disables the button. Acceptable for a manual LAN action; parallelizing is a noted future tweak.
