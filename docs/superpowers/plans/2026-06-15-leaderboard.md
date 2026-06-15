# Per-Repo Leaderboard — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`).

**Goal:** Make the bake-off *accrue*: every judged comparison records each model's result (won? tests passed?) for that repo, and a Leaderboard view shows "for this repo, model X wins N/M, tests Y%." Zero extra steps — it records automatically on judge.

**Architecture:** A `leaderboard` module owns a JSON store of bake-off records + pure aggregation. The judge endpoint auto-records on a successful verdict (parsing a structured `WINNER:` line) and returns the winner so the UI can highlight it. A `GET /leaderboard` endpoint + a dashboard panel surface the standings.

**UX wins folded in:** auto-recording (no manual logging), a 🏆 highlight on the winning compare column, and the leaderboard reachable from both the toolbar and the comparison view.

---

### Task 1: `leaderboard` module (types + store + aggregation)

**Files:** create `crates/kaiju-daemon/src/leaderboard.rs`; `lib.rs` (`pub mod leaderboard;`).

- [ ] **Step 1: Create `leaderboard.rs`:**
```rust
//! Per-repo bake-off history: which model won, whose tests passed. Persisted to
//! `~/.kaiju/leaderboard.json` (override `KAIJU_LEADERBOARD`). Mirrors the
//! settings/devices stores: optional file, best-effort load, explicit save.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use kaiju_core::{NexusError, Result};
use serde::{Deserialize, Serialize};

/// One candidate's outcome within a judged bake-off.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct CandidateResult {
    pub agent_type: String,
    #[serde(default)]
    pub model: Option<String>,
    /// `Some(true/false)` when a test command was run; `None` if diff-only.
    #[serde(default)]
    pub tests_passed: Option<bool>,
    #[serde(default)]
    pub is_winner: bool,
}

/// One judged bake-off (a compare group that was judged).
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct BakeoffRecord {
    pub workspace: String,
    pub prompt: String,
    pub at: DateTime<Utc>,
    pub candidates: Vec<CandidateResult>,
}

/// Aggregated standing for one model in a workspace.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct AgentStanding {
    pub agent_type: String,
    pub runs: u32,
    pub wins: u32,
    pub tests_passed: u32,
    pub tests_run: u32,
}

/// Pure: standings for `workspace`, sorted by wins then tests-passed, descending.
pub fn standings(records: &[BakeoffRecord], workspace: &str) -> Vec<AgentStanding> {
    use std::collections::BTreeMap;
    let mut map: BTreeMap<String, AgentStanding> = BTreeMap::new();
    for rec in records.iter().filter(|r| r.workspace == workspace) {
        for c in &rec.candidates {
            let s = map.entry(c.agent_type.clone()).or_insert_with(|| AgentStanding {
                agent_type: c.agent_type.clone(),
                runs: 0,
                wins: 0,
                tests_passed: 0,
                tests_run: 0,
            });
            s.runs += 1;
            if c.is_winner {
                s.wins += 1;
            }
            if let Some(passed) = c.tests_passed {
                s.tests_run += 1;
                if passed {
                    s.tests_passed += 1;
                }
            }
        }
    }
    let mut v: Vec<AgentStanding> = map.into_values().collect();
    v.sort_by(|a, b| b.wins.cmp(&a.wins).then(b.tests_passed.cmp(&a.tests_passed)));
    v
}

fn leaderboard_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("KAIJU_LEADERBOARD") {
        return Some(PathBuf::from(path));
    }
    std::env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join(".kaiju").join("leaderboard.json"))
}

/// Read all records; an absent/malformed file means "no history".
pub fn load() -> Vec<BakeoffRecord> {
    let Some(path) = leaderboard_path() else { return Vec::new() };
    let Ok(content) = std::fs::read_to_string(path) else { return Vec::new() };
    serde_json::from_str(&content).unwrap_or_default()
}

/// Append one record (best-effort persist).
pub fn record(entry: BakeoffRecord) -> Result<()> {
    let path = leaderboard_path().ok_or_else(|| {
        NexusError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no leaderboard path (set KAIJU_LEADERBOARD or HOME)",
        ))
    })?;
    let mut all = load();
    all.push(entry);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(NexusError::Io)?;
    }
    let json = serde_json::to_string_pretty(&all)
        .map_err(|e| NexusError::Io(std::io::Error::other(e.to_string())))?;
    std::fs::write(&path, json).map_err(NexusError::Io)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at() -> DateTime<Utc> { "2026-06-15T00:00:00Z".parse().unwrap() }

    fn rec(ws: &str, cands: Vec<CandidateResult>) -> BakeoffRecord {
        BakeoffRecord { workspace: ws.into(), prompt: "p".into(), at: at(), candidates: cands }
    }
    fn cand(t: &str, win: bool, tests: Option<bool>) -> CandidateResult {
        CandidateResult { agent_type: t.into(), model: None, tests_passed: tests, is_winner: win }
    }

    #[test]
    fn standings_aggregate_wins_and_tests_for_workspace() {
        let records = vec![
            rec("/repo", vec![cand("claude", true, Some(true)), cand("codex", false, Some(false))]),
            rec("/repo", vec![cand("claude", false, Some(false)), cand("codex", true, Some(true))]),
            rec("/other", vec![cand("claude", true, None)]), // different workspace — excluded
        ];
        let s = standings(&records, "/repo");
        assert_eq!(s.len(), 2);
        // tie on wins (1 each) -> ordered by tests_passed; both have 1, stable.
        let claude = s.iter().find(|x| x.agent_type == "claude").unwrap();
        assert_eq!(claude.runs, 2);
        assert_eq!(claude.wins, 1);
        assert_eq!(claude.tests_run, 2);
        assert_eq!(claude.tests_passed, 1);
    }

    #[test]
    fn standings_empty_for_unknown_workspace() {
        assert!(standings(&[], "/nope").is_empty());
    }
}
```
Add `pub mod leaderboard;` to `lib.rs` (alphabetical — after `judge`/before `logstore`).

- [ ] **Step 2: Verify** — `cargo test -p kaiju-daemon leaderboard::` → 2 pass. `cargo build -p kaiju-daemon` (dead_code on `record` until Task 3 — expected).

- [ ] **Step 3: Commit**
```bash
git add crates/kaiju-daemon/src/leaderboard.rs crates/kaiju-daemon/src/lib.rs
git commit -m "feat(daemon): leaderboard store + standings aggregation"
```

---

### Task 2: structured winner in the judge

**Files:** `crates/kaiju-daemon/src/judge.rs`

- [ ] **Step 1: Failing test** — append to `mod tests`:
```rust
    #[test]
    fn parse_winner_reads_trailing_marker() {
        assert_eq!(parse_winner("blah\nWINNER: B"), Some("B".to_string()));
        assert_eq!(parse_winner("WINNER:  aa  \ntrailing"), Some("AA".to_string()));
        assert_eq!(parse_winner("no marker here"), None);
    }
```

- [ ] **Step 2: Add `parse_winner` + instruct the prompt.** Add the function:
```rust
/// Extract the structured winner label from a verdict ending with `WINNER: X`.
/// Scans from the end and reads the trailing alphabetic label.
pub fn parse_winner(verdict: &str) -> Option<String> {
    verdict.lines().rev().find_map(|line| {
        let rest = line.trim().strip_prefix("WINNER:")?;
        let label: String = rest.trim().chars().take_while(|c| c.is_ascii_alphabetic()).collect();
        (!label.is_empty()).then(|| label.to_uppercase())
    })
}
```
In `build_prompt`, change the closing instruction so the model emits the marker. Replace `"...then name the winner. Be concise.\n\n## Task\n"` so it reads (keep the rest of the string intact):
```
...then name the winner. End your reply with a line exactly `WINNER: <letter>`. Be concise.
```
(The marker uses a letter label, not a CLI name — the anonymization test `prompt_is_anonymized_and_has_task` still holds.)

- [ ] **Step 3: Verify** — `cargo test -p kaiju-daemon judge::` → all pass (incl. parse_winner + the unchanged anonymization test). `cargo clippy -p kaiju-daemon` clean.

- [ ] **Step 4: Commit**
```bash
git add crates/kaiju-daemon/src/judge.rs
git commit -m "feat(judge): structured WINNER marker + parse_winner"
```

---

### Task 3: record on judge + `GET /leaderboard`

**Files:** `crates/kaiju-daemon/src/api.rs`, `crates/kaiju-daemon/tests/api.rs`

- [ ] **Step 1: Record after a successful judge.** In `compare_judge`, replace the final `match ... run_judge` arm so the `Ok` branch parses the winner, builds + saves a `BakeoffRecord`, and returns the winner. Replace:
```rust
    match crate::judge::run_judge(&workspace, &prompt).await {
        Ok(verdict) => Ok(Json(serde_json::json!({ "verdict": verdict, "legend": legend }))),
        Err(e) => Err(err(StatusCode::BAD_GATEWAY, &format!("judge unavailable: {e}"))),
    }
```
with:
```rust
    match crate::judge::run_judge(&workspace, &prompt).await {
        Ok(verdict) => {
            let winner_label = crate::judge::parse_winner(&verdict);
            // Build a leaderboard record from the (ordered) agents + candidates.
            let results: Vec<crate::leaderboard::CandidateResult> = agents
                .iter()
                .zip(candidates.iter())
                .enumerate()
                .map(|(i, (a, c))| crate::leaderboard::CandidateResult {
                    agent_type: a.agent_type.to_string(),
                    model: a.model.clone(),
                    tests_passed: c.test.as_ref().map(|t| t.passed),
                    is_winner: winner_label.as_deref() == Some(crate::judge::label_for(i).as_str()),
                })
                .collect();
            let _ = crate::leaderboard::record(crate::leaderboard::BakeoffRecord {
                workspace: workspace.display().to_string(),
                prompt: task.clone(),
                at: chrono::Utc::now(),
                candidates: results,
            });
            Ok(Json(serde_json::json!({
                "verdict": verdict,
                "legend": legend,
                "winner": winner_label,
            })))
        }
        Err(e) => Err(err(StatusCode::BAD_GATEWAY, &format!("judge unavailable: {e}"))),
    }
```
(`candidates` is still in scope — `build_prompt` borrowed it. `c.test` is `Option<TestSummary>`, read it here.)

- [ ] **Step 2: `GET /leaderboard`.** Add the query type + route + handler. Type:
```rust
#[derive(Deserialize)]
pub struct LeaderboardQuery {
    pub workspace: String,
}
```
Route (near `/compare/judge`): `.route("/leaderboard", get(leaderboard))`
Handler:
```rust
/// `GET /leaderboard?workspace=<path>` — model standings for that repo.
async fn leaderboard(
    axum::extract::Query(q): axum::extract::Query<LeaderboardQuery>,
) -> impl IntoResponse {
    let records = crate::leaderboard::load();
    Json(crate::leaderboard::standings(&records, &q.workspace))
}
```

- [ ] **Step 3: Test** — append to `tests/api.rs`:
```rust
#[tokio::test]
async fn leaderboard_empty_for_unknown_workspace() {
    let app = build_app(AppState::new());
    let resp = app.oneshot(get_request("/leaderboard?workspace=/no/such/repo")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json.as_array().unwrap().is_empty());
}
```
(Isolate the store so the test can't read a real file: set `KAIJU_LEADERBOARD` to a temp path at the top of the test — mirror the `isolate_devices_path` helper pattern used elsewhere; a missing file yields `[]`.)

- [ ] **Step 4: Verify** — `cargo test -p kaiju-daemon`; `cargo clippy -p kaiju-daemon` (clean — leaderboard dead_code resolved); `cargo build`.

- [ ] **Step 5: Commit**
```bash
git add crates/kaiju-daemon/src/api.rs crates/kaiju-daemon/tests/api.rs
git commit -m "feat(daemon): auto-record bake-offs + GET /leaderboard"
```

---

### Task 4: Leaderboard UI + winner highlight

**Files:** `crates/kaiju-daemon/src/dashboard.rs`, `crates/kaiju-daemon/assets/dashboard.js`

- [ ] **Step 1: Toolbar button + panel (dashboard.rs).** Add a button beside "Compare task":
```html
    <button onclick="openLeaderboard()">Leaderboard</button>
```
Add a panel (sibling to `#compare-panel`):
```html
  <div id="leaderboard-panel" class="card" hidden>
    <div class="detail-head">
      <span class="id">Leaderboard</span>
      <input id="lb-ws" placeholder="/path/to/repo" autocomplete="off" style="flex:0 1 22rem" onchange="loadLeaderboard()">
      <span class="grow"></span>
      <button onclick="closeLeaderboard()">Close</button>
    </div>
    <div id="lb-body"></div>
  </div>
```
CSS (style block): `#leaderboard-panel { margin-top:1.5rem; padding:1.1rem 1.25rem; } #leaderboard-panel[hidden]{display:none}` plus reuse the existing `table` styles.

- [ ] **Step 2: JS — leaderboard view (append to dashboard.js).**
```javascript
function openLeaderboard() {
  document.getElementById("detail").hidden = true;
  document.getElementById("leaderboard-panel").hidden = false;
  const ws = document.getElementById("lb-ws");
  if (!ws.value) ws.value = localStorage.getItem("kaiju_cmp_ws") || "";
  loadLeaderboard();
}
function closeLeaderboard() { document.getElementById("leaderboard-panel").hidden = true; }

async function loadLeaderboard() {
  const ws = document.getElementById("lb-ws").value.trim();
  const body = document.getElementById("lb-body");
  if (!ws) { body.innerHTML = '<div class="empty">Enter a repo path.</div>'; return; }
  try {
    const rows = await (await api("/leaderboard?workspace=" + encodeURIComponent(ws))).json();
    if (!rows.length) { body.innerHTML = '<div class="empty">No judged bake-offs for this repo yet.</div>'; return; }
    body.innerHTML = '<table><thead><tr><th>Model</th><th>Wins</th><th>Runs</th><th>Tests passed</th></tr></thead><tbody>' +
      rows.map(function (r) {
        const tests = r.tests_run ? (r.tests_passed + "/" + r.tests_run) : "—";
        return '<tr><td>' + esc(r.agent_type) + '</td><td>' + r.wins + '</td><td>' + r.runs + '</td><td>' + tests + '</td></tr>';
      }).join("") + '</tbody></table>';
  } catch (e) { body.innerHTML = '<div class="empty">Could not load the leaderboard.</div>'; }
}
```

- [ ] **Step 3: Remember the compare workspace + highlight the winner.**
In `submitCompare()` (the compare-modal submit), after reading `ws`, persist it so the leaderboard prefills: `localStorage.setItem("kaiju_cmp_ws", ws);`
In `judgeComparison()`, after rendering the verdict, mark the winning column. The judge response now includes `winner` (a label); map it to an agent id via the legend and badge that column. Add, after `box.innerHTML = ...`:
```javascript
    if (d.winner && d.legend) {
      const w = d.legend.find(function (l) { return l.label === d.winner; });
      if (w) {
        const head = document.querySelector('.cmp-col[data-id="' + w.id + '"] .cmp-col-head');
        if (head && !head.querySelector(".trophy")) {
          const t = document.createElement("span");
          t.className = "trophy"; t.textContent = "🏆"; t.title = "Judge's winner";
          head.insertBefore(t, head.firstChild);
        }
      }
    }
```

- [ ] **Step 4: Verify** — `node --check dashboard.js`; `node --test dashboard-utils.test.js`; `cargo build -p kaiju-daemon`; `cargo test -p kaiju-daemon`.

- [ ] **Step 5: Commit**
```bash
git add crates/kaiju-daemon/src/dashboard.rs crates/kaiju-daemon/assets/dashboard.js
git commit -m "feat(dashboard): leaderboard view + winner highlight"
```

---

### Task 5: Docs

**Files:** `README.md`

- [ ] **Step 1:** Add to the bake-off section:
```markdown
- **Leaderboard.** Every judged bake-off is recorded per repo — open **Leaderboard**
  to see which model wins most and whose tests pass most for *this* codebase.
```
Add the API row:
```markdown
| GET | `/leaderboard?workspace=` | Per-repo model standings (wins, runs, test pass-rate). |
```

- [ ] **Step 2: Commit**
```bash
git add README.md
git commit -m "docs: per-repo leaderboard"
```

---

## Self-Review Notes
- **Spec coverage:** store + standings (T1), structured winner (T2), auto-record + endpoint (T3), UI + highlight (T4), docs (T5).
- **Auto-record is best-effort** (`let _ = record(...)`) so a write failure never fails the judge response.
- **Winner is objective-friendly:** if the model omits the `WINNER:` marker, `winner_label` is `None` → no win credited, but per-candidate test pass/fail is still recorded, so the leaderboard's test columns stay meaningful.
- **Type consistency:** `CandidateResult`/`BakeoffRecord`/`AgentStanding` (leaderboard) ↔ the handler's record build ↔ the JS table fields (`agent_type`, `wins`, `runs`, `tests_passed`, `tests_run`). `winner` label ↔ legend `label`/`id` for the highlight.
- **Test isolation:** the leaderboard endpoint test sets `KAIJU_LEADERBOARD` to a temp path so it never reads/writes the real history.
