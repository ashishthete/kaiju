# Auto-Continue on Usage Limit — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When a Claude Code agent hits its usage/session limit, detect it, parse the reset time, and auto-inject `continue` into the live tmux session at reset — for agents that opted in.

**Architecture:** The Claude adapter recognizes the limit banner (pure) and extracts the reset time + IANA timezone. The existing `monitor.rs` poll loop resolves the new `AgentStatus::Limited`, arms a per-agent `resume_at` timer via a pure `reset_to_utc`, and injects `continue` when due. Opt-in is a per-agent flag with a daemon-wide default, mirroring the existing `isolate` plumbing.

**Tech Stack:** Rust (axum daemon, tmux via `Command`), `chrono` + new `chrono-tz` for timezone math, vanilla JS dashboard with node `--test` unit tests.

## Global Constraints

- No `unwrap()`/`expect()`/`panic!()` outside tests, startup validation, or `LazyLock` regex compiles of constant literals (matches existing `claude.rs` pattern). Use `Result`/`Option`.
- Follow the existing `isolate` field as the template for all opt-in plumbing (config field → Agent field → Settings default → request body → dashboard checkbox).
- Ship gate before merge (per repo convention): `cargo fmt --all && cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace`, `node --test crates/kaiju-daemon/assets/`.
- Reset-time format to target (verbatim real sample): `You've hit your session limit · resets 5:30pm (Asia/Calcutta)`.
- The daemon runs as `./target/release/kaiju-daemon` with env `KAIJU_HOST=0.0.0.0 KAIJU_DESKTOP_NOTIFY=1`; do NOT restart it as part of this plan (the user has live sessions).

---

## File Structure

- `Cargo.toml` (workspace) — add `chrono-tz` to `[workspace.dependencies]`.
- `crates/kaiju-adapters/Cargo.toml` — depend on `chrono-tz`.
- `crates/kaiju-adapters/src/limit.rs` **(new)** — pure `parse_clock` + `reset_to_utc` (timezone math, isolated so its `chrono-tz` use is self-contained and unit-testable).
- `crates/kaiju-adapters/src/lib.rs` — `pub mod limit;`.
- `crates/kaiju-core/src/agent.rs` — `AgentStatus::Limited`; `Agent` gains `auto_continue`, `resume_at`, `last_fired`.
- `crates/kaiju-core/src/adapter.rs` — `ParsedOutput` gains `limit_reset`.
- `crates/kaiju-adapters/src/claude.rs` — banner detection in `parse_output`.
- `crates/kaiju-daemon/src/settings.rs` — `Settings.auto_continue` default.
- `crates/kaiju-daemon/src/store.rs` — `set_resume_at`, `mark_auto_continue_fired`.
- `crates/kaiju-daemon/src/api.rs` — `CreateAgentRequest.auto_continue`, wire onto agent; `AgentResponse.resume_at`.
- `crates/kaiju-daemon/src/server.rs` — `spawn_started_agent` sets `auto_continue` from defaults.
- `crates/kaiju-daemon/src/monitor.rs` — `should_fire` pure fn + arm/fire/disarm in `poll_once`.
- `crates/kaiju-daemon/assets/dashboard-utils.js` + `dashboard-utils.test.js` — `limited` status maps.
- `crates/kaiju-daemon/src/dashboard.rs` — `.s-limited` CSS, New-Agent + Preferences checkboxes.
- `crates/kaiju-daemon/assets/dashboard.js` — send/read `auto_continue`, "resumes 5:30pm" badge.

---

## Task 1: Pure reset-time resolver (`limit.rs` + chrono-tz)

**Files:**
- Modify: `Cargo.toml` (workspace `[workspace.dependencies]`)
- Modify: `crates/kaiju-adapters/Cargo.toml`
- Create: `crates/kaiju-adapters/src/limit.rs`
- Modify: `crates/kaiju-adapters/src/lib.rs`

**Interfaces:**
- Produces:
  - `pub fn parse_clock(s: &str) -> Option<(u32, u32)>` — `(hour_0_23, minute)`.
  - `pub fn reset_to_utc(now: chrono::DateTime<chrono::Utc>, time_str: &str, tz_name: &str) -> Option<chrono::DateTime<chrono::Utc>>`.

- [ ] **Step 1: Add the dependency.** In `Cargo.toml` under `[workspace.dependencies]` (near the existing `chrono = ...` line) add:

```toml
chrono-tz = "0.9"
```

In `crates/kaiju-adapters/Cargo.toml` under `[dependencies]` (below `chrono = { workspace = true }`) add:

```toml
chrono-tz = { workspace = true }
```

- [ ] **Step 2: Write the failing test.** Create `crates/kaiju-adapters/src/limit.rs` with only the tests (functions not yet defined):

```rust
//! Pure helpers to turn a Claude limit banner's reset time into a UTC instant.
//! No IO — fully unit-tested. `chrono-tz` resolves the IANA zone name the banner
//! prints (e.g. "Asia/Calcutta").

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    #[test]
    fn parse_clock_handles_am_pm_and_minutes() {
        assert_eq!(parse_clock("3pm"), Some((15, 0)));
        assert_eq!(parse_clock("5:30pm"), Some((17, 30)));
        assert_eq!(parse_clock("9:05AM"), Some((9, 5)));
        assert_eq!(parse_clock("12am"), Some((0, 0)));
        assert_eq!(parse_clock("12pm"), Some((12, 0)));
        assert_eq!(parse_clock("nonsense"), None);
        assert_eq!(parse_clock("13pm"), None);
    }

    #[test]
    fn reset_future_time_resolves_to_utc() {
        // 09:00 UTC = 14:30 IST. Reset 5:30pm IST (17:30) = 12:00 UTC.
        let now = Utc.with_ymd_and_hms(2026, 7, 12, 9, 0, 0).unwrap();
        let got = reset_to_utc(now, "5:30pm", "Asia/Calcutta").unwrap();
        assert_eq!(got, Utc.with_ymd_and_hms(2026, 7, 12, 12, 0, 0).unwrap());
    }

    #[test]
    fn reset_past_time_resolves_to_now() {
        // 13:00 UTC = 18:30 IST, already past the 17:30 IST reset → fire now.
        let now = Utc.with_ymd_and_hms(2026, 7, 12, 13, 0, 0).unwrap();
        assert_eq!(reset_to_utc(now, "5:30pm", "Asia/Calcutta").unwrap(), now);
    }

    #[test]
    fn reset_bad_timezone_is_none() {
        let now = Utc.with_ymd_and_hms(2026, 7, 12, 9, 0, 0).unwrap();
        assert_eq!(reset_to_utc(now, "5:30pm", "Not/AZone"), None);
    }
}
```

Add `pub mod limit;` to `crates/kaiju-adapters/src/lib.rs` (alongside the other `pub mod` lines).

- [ ] **Step 3: Run test to verify it fails.**

Run: `cargo test -p kaiju-adapters limit`
Expected: FAIL — `cannot find function parse_clock` / `reset_to_utc`.

- [ ] **Step 4: Write the implementation.** Prepend to `crates/kaiju-adapters/src/limit.rs` (above the `#[cfg(test)]` module):

```rust
use chrono::{DateTime, Datelike, TimeZone, Utc};
use chrono_tz::Tz;

/// Parse a 12-hour clock like "5:30pm" / "3pm" into `(hour_0_23, minute)`.
/// Case-insensitive; minutes optional. Returns `None` on anything unexpected.
pub fn parse_clock(s: &str) -> Option<(u32, u32)> {
    let s = s.trim().to_lowercase();
    let (body, pm) = if let Some(b) = s.strip_suffix("pm") {
        (b, true)
    } else if let Some(b) = s.strip_suffix("am") {
        (b, false)
    } else {
        return None;
    };
    let body = body.trim();
    let (h, m) = match body.split_once(':') {
        Some((h, m)) => (h.trim().parse::<u32>().ok()?, m.trim().parse::<u32>().ok()?),
        None => (body.parse::<u32>().ok()?, 0),
    };
    if h == 0 || h > 12 || m > 59 {
        return None;
    }
    let hour24 = match (h, pm) {
        (12, false) => 0,
        (12, true) => 12,
        (h, false) => h,
        (h, true) => h + 12,
    };
    Some((hour24, m))
}

/// The UTC instant of the next occurrence of wall-clock `time_str` in IANA
/// `tz_name`. Uses today's date in that zone; if that instant already passed
/// `now` (clock skew / a stale banner), returns `now` so we fire immediately.
/// Claude session resets are intra-day, so we never roll forward a whole day.
pub fn reset_to_utc(now: DateTime<Utc>, time_str: &str, tz_name: &str) -> Option<DateTime<Utc>> {
    let (hour, minute) = parse_clock(time_str)?;
    let tz: Tz = tz_name.parse().ok()?;
    let local = now.with_timezone(&tz);
    let target = tz
        .with_ymd_and_hms(local.year(), local.month(), local.day(), hour, minute, 0)
        .single()?
        .with_timezone(&Utc);
    Some(if target <= now { now } else { target })
}
```

- [ ] **Step 5: Run tests to verify they pass.**

Run: `cargo test -p kaiju-adapters limit`
Expected: PASS (4 tests).

- [ ] **Step 6: Commit.**

```bash
git add Cargo.toml Cargo.lock crates/kaiju-adapters/Cargo.toml crates/kaiju-adapters/src/limit.rs crates/kaiju-adapters/src/lib.rs
git commit -m "feat(adapters): pure reset-time resolver (chrono-tz)"
```

---

## Task 2: `AgentStatus::Limited` variant

**Files:**
- Modify: `crates/kaiju-core/src/agent.rs:41-70` (enum + `is_terminal`/`is_active`)

**Interfaces:**
- Produces: `AgentStatus::Limited` — `is_terminal()==false`, `is_active()==true`. Serializes (snake_case) to `"limited"`.

- [ ] **Step 1: Write the failing test.** In the `#[cfg(test)] mod tests` of `agent.rs`, add:

```rust
#[test]
fn limited_is_active_not_terminal() {
    assert!(AgentStatus::Limited.is_active());
    assert!(!AgentStatus::Limited.is_terminal());
}
```

- [ ] **Step 2: Run test to verify it fails.**

Run: `cargo test -p kaiju-core limited_is_active`
Expected: FAIL — `no variant named Limited`.

- [ ] **Step 3: Add the variant and update the predicates.** In the enum add `Limited` after `Stuck`:

```rust
pub enum AgentStatus {
    Starting,
    Running,
    WaitingForInput,
    Error,
    Completed,
    Stuck,
    Limited,
    Stopped,
}
```

In `is_active`, add `Limited` to the matched set:

```rust
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            AgentStatus::Starting
                | AgentStatus::Running
                | AgentStatus::WaitingForInput
                | AgentStatus::Stuck
                | AgentStatus::Limited
        )
    }
```

`is_terminal` is unchanged (Limited is not terminal).

- [ ] **Step 4: Verify the whole workspace still compiles** (catches any exhaustive `match` on `AgentStatus`; the known ones in `scheduler.rs` and `monitor.rs` use wildcards, and `AgentResponse` uses `{:?}` formatting, so none should break):

Run: `cargo build --workspace`
Expected: builds clean.

- [ ] **Step 5: Run tests to verify they pass.**

Run: `cargo test -p kaiju-core limited_is_active`
Expected: PASS.

- [ ] **Step 6: Commit.**

```bash
git add crates/kaiju-core/src/agent.rs
git commit -m "feat(core): add AgentStatus::Limited"
```

---

## Task 3: Detect the limit banner in the Claude adapter

**Files:**
- Modify: `crates/kaiju-core/src/adapter.rs:17-20` (`ParsedOutput`)
- Modify: `crates/kaiju-adapters/src/limit.rs` (add `parse_limit_reset`)
- Modify: `crates/kaiju-adapters/src/claude.rs:57-91` (`parse_output`)

**Interfaces:**
- Consumes: `AgentStatus::Limited` (Task 2).
- Produces:
  - `ParsedOutput.limit_reset: Option<(String, String)>` — `(time, tz)`, e.g. `("5:30pm","Asia/Calcutta")`.
  - `pub fn parse_limit_reset(output: &str) -> Option<(String, String)>` in `limit.rs`.

- [ ] **Step 1: Add the `ParsedOutput` field.** In `crates/kaiju-core/src/adapter.rs`, add to the struct (it derives `Default`, so `None` is the default):

```rust
pub struct ParsedOutput {
    pub status: Option<AgentStatus>,
    pub tokens_used: Option<u64>,
    pub estimated_cost_usd: Option<f64>,
    /// `(reset_time, iana_timezone)` parsed from a usage-limit banner, e.g.
    /// `("5:30pm", "Asia/Calcutta")`. `None` unless the agent is limited.
    pub limit_reset: Option<(String, String)>,
}
```

- [ ] **Step 2: Write the failing tests.** In `crates/kaiju-adapters/src/claude.rs` test module add:

```rust
    #[test]
    fn session_limit_banner_sets_limited() {
        let out = "You've hit your session limit \u{b7} resets 5:30pm (Asia/Calcutta)\n\
                   /upgrade or /usage-credits to finish what you're working on.\n\n\u{273b}";
        let parsed = ClaudeAdapter.parse_output(out);
        assert_eq!(parsed.status, Some(AgentStatus::Limited));
        assert_eq!(
            parsed.limit_reset,
            Some(("5:30pm".to_string(), "Asia/Calcutta".to_string()))
        );
    }

    #[test]
    fn plain_error_line_is_not_limited() {
        let parsed = ClaudeAdapter.parse_output("Error: API rate limit exceeded");
        assert_eq!(parsed.status, Some(AgentStatus::Error));
        assert_eq!(parsed.limit_reset, None);
    }
```

In `crates/kaiju-adapters/src/limit.rs` test module add:

```rust
    #[test]
    fn parse_limit_reset_extracts_time_and_zone() {
        let out = "You've hit your session limit \u{b7} resets 5:30pm (Asia/Calcutta)";
        assert_eq!(
            parse_limit_reset(out),
            Some(("5:30pm".to_string(), "Asia/Calcutta".to_string()))
        );
        assert_eq!(parse_limit_reset("just some output"), None);
    }
```

- [ ] **Step 3: Run tests to verify they fail.**

Run: `cargo test -p kaiju-adapters limit_reset && cargo test -p kaiju-adapters session_limit`
Expected: FAIL — `parse_limit_reset` undefined; `Limited` not produced.

- [ ] **Step 4: Implement `parse_limit_reset`.** In `crates/kaiju-adapters/src/limit.rs`, add near the top (after the `use` lines):

```rust
use regex::Regex;
use std::sync::LazyLock;

/// Matches `resets 5:30pm (Asia/Calcutta)` in a limit banner (case-insensitive).
static LIMIT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)resets\s+(\d{1,2}(?::\d{2})?\s*[ap]m)\s*\(([^)]+)\)").unwrap()
});

/// Extract `(reset_time, iana_timezone)` from a limit banner, or `None`.
pub fn parse_limit_reset(output: &str) -> Option<(String, String)> {
    let caps = LIMIT_RE.captures(output)?;
    Some((caps[1].trim().to_string(), caps[2].trim().to_string()))
}
```

(`regex` is already a dependency of this crate — see `claude.rs`.)

- [ ] **Step 5: Detect the banner in `parse_output`.** In `crates/kaiju-adapters/src/claude.rs`, at the very top of `parse_output` (before `let last = ...`), insert:

```rust
    fn parse_output(&self, output: &str) -> ParsedOutput {
        let mut result = ParsedOutput::default();

        // A usage/session limit takes priority: the session is alive but stalled
        // until reset. Detect it before the waiting/error branches so the prompt
        // line that sits below the banner doesn't mask it.
        if output.contains("session limit") || output.contains("usage limit") {
            if let Some(reset) = crate::limit::parse_limit_reset(output) {
                result.status = Some(AgentStatus::Limited);
                result.limit_reset = Some(reset);
                return result;
            }
        }

        let last = last_non_empty_line(output);
        // ... existing body unchanged ...
```

- [ ] **Step 6: Run tests to verify they pass.**

Run: `cargo test -p kaiju-adapters`
Expected: PASS (new + existing adapter tests, including the untouched `error_line` cases).

- [ ] **Step 7: Commit.**

```bash
git add crates/kaiju-core/src/adapter.rs crates/kaiju-adapters/src/limit.rs crates/kaiju-adapters/src/claude.rs
git commit -m "feat(adapters): detect Claude usage-limit banner -> Limited"
```

---

## Task 4: Opt-in flag, runtime scheduling fields, and API exposure

**Files:**
- Modify: `crates/kaiju-core/src/agent.rs` (`Agent` struct + `Agent::new`)
- Modify: `crates/kaiju-daemon/src/settings.rs` (`Settings`)
- Modify: `crates/kaiju-daemon/src/store.rs` (mutators)
- Modify: `crates/kaiju-daemon/src/api.rs` (`CreateAgentRequest`, `create_agent`, `AgentResponse`)
- Modify: `crates/kaiju-daemon/src/server.rs` (`spawn_started_agent`)

**Interfaces:**
- Consumes: nothing new.
- Produces:
  - `Agent.auto_continue: bool`, `Agent.resume_at: Option<DateTime<Utc>>`, `Agent.last_fired: Option<DateTime<Utc>>`.
  - `Settings.auto_continue: bool`.
  - `Store::set_resume_at(&self, id: &str, resume_at: Option<DateTime<Utc>>) -> bool`.
  - `Store::mark_auto_continue_fired(&self, id: &str, now: DateTime<Utc>) -> bool`.
  - `AgentResponse.resume_at: Option<String>` (RFC3339).

- [ ] **Step 1: Add fields to `Agent`.** In `crates/kaiju-core/src/agent.rs`, in the `Agent` struct add after `pub metrics: AgentMetrics,` (keep it last-ish; all `#[serde(default)]` so old persisted state still loads):

```rust
    pub metrics: AgentMetrics,
    /// Auto-inject `continue` at reset when this agent hits a usage limit.
    #[serde(default)]
    pub auto_continue: bool,
    /// When armed, the UTC instant to fire the auto-continue at (from the limit
    /// banner's reset time). `None` when not limited / not armed.
    #[serde(default)]
    pub resume_at: Option<DateTime<Utc>>,
    /// Last time auto-continue fired, for the re-fire backoff.
    #[serde(default)]
    pub last_fired: Option<DateTime<Utc>>,
}
```

In `Agent::new`, add the three initializers after `metrics: AgentMetrics::default(),`:

```rust
            metrics: AgentMetrics::default(),
            auto_continue: false,
            resume_at: None,
            last_fired: None,
        }
```

- [ ] **Step 2: Build to confirm the struct change compiles.**

Run: `cargo build -p kaiju-core`
Expected: builds clean.

- [ ] **Step 3: Add the Settings default + test.** In `crates/kaiju-daemon/src/settings.rs`, add to `Settings` after the `isolate` field:

```rust
    /// Run agents in an isolated git worktree by default.
    #[serde(default)]
    pub isolate: bool,
    /// Auto-continue agents past a usage limit by default.
    #[serde(default)]
    pub auto_continue: bool,
```

Add a test in that file's test module:

```rust
    #[test]
    fn auto_continue_defaults_off() {
        assert!(!Settings::default().auto_continue);
    }
```

Run: `cargo test -p kaiju-daemon auto_continue_defaults_off`
Expected: PASS.

- [ ] **Step 4: Add store mutators + test.** In `crates/kaiju-daemon/src/store.rs`, after `update_metrics`, add:

```rust
    /// Arm (or clear) the auto-continue timer for an agent.
    pub fn set_resume_at(&self, id: &str, resume_at: Option<DateTime<Utc>>) -> bool {
        let updated = {
            let mut agents = self.agents.write().unwrap();
            match agents.get_mut(id) {
                Some(agent) => {
                    agent.resume_at = resume_at;
                    true
                }
                None => false,
            }
        };
        if updated {
            self.persist();
        }
        updated
    }

    /// Record an auto-continue fire: clear the timer, stamp the fire time.
    pub fn mark_auto_continue_fired(&self, id: &str, now: DateTime<Utc>) -> bool {
        let updated = {
            let mut agents = self.agents.write().unwrap();
            match agents.get_mut(id) {
                Some(agent) => {
                    agent.resume_at = None;
                    agent.last_fired = Some(now);
                    true
                }
                None => false,
            }
        };
        if updated {
            self.persist();
        }
        updated
    }
```

Add a test in `store.rs`'s test module. The store type is `AgentStore`, and the module already has a `test_agent(AgentType) -> Agent` helper and constructs with `AgentStore::new()` (see the existing tests). Add `use chrono::Utc;` to the test module if not already imported:

```rust
    #[test]
    fn set_resume_at_and_fire_roundtrip() {
        let store = AgentStore::new();
        let agent = test_agent(AgentType::Claude);
        let id = agent.id.clone();
        store.insert(agent);

        let t = Utc::now();
        assert!(store.set_resume_at(&id, Some(t)));
        assert_eq!(store.get(&id).unwrap().resume_at, Some(t));

        assert!(store.mark_auto_continue_fired(&id, t));
        let a = store.get(&id).unwrap();
        assert_eq!(a.resume_at, None);
        assert_eq!(a.last_fired, Some(t));
    }
```

Run: `cargo test -p kaiju-daemon set_resume_at_and_fire_roundtrip`
Expected: PASS.

- [ ] **Step 5: Accept `auto_continue` in the create request and set it on the agent.** In `crates/kaiju-daemon/src/api.rs`, add to `CreateAgentRequest` (after `pub auto_approve: bool,`):

```rust
    #[serde(default)]
    pub auto_approve: bool,
    /// If true, auto-continue this agent past a usage limit at reset time.
    #[serde(default)]
    pub auto_continue: bool,
}
```

In `create_agent`, after `agent.batch = req.batch;` add:

```rust
    agent.batch = req.batch;
    agent.auto_continue = req.auto_continue || defaults.auto_continue;
```

- [ ] **Step 6: Default `auto_continue` in the shared spawn path.** In `crates/kaiju-daemon/src/server.rs`, in `spawn_started_agent`, after `agent.isolate = isolate;` add:

```rust
    agent.isolate = isolate;
    agent.auto_continue = defaults.auto_continue;
```

- [ ] **Step 7: Expose `resume_at` on the API.** In `crates/kaiju-daemon/src/api.rs`, add to `AgentResponse` (after `pub port: Option<u16>,`):

```rust
    pub port: Option<u16>,
    /// When auto-continue will fire (RFC3339), if armed. Drives the dashboard badge.
    pub resume_at: Option<String>,
}
```

In `impl From<&Agent> for AgentResponse`, add after `port: agent.port,`:

```rust
            port: agent.port,
            resume_at: agent.resume_at.map(|t| t.to_rfc3339()),
```

- [ ] **Step 8: Build + run the daemon test suite.**

Run: `cargo test -p kaiju-daemon`
Expected: PASS (all existing + the two new tests).

- [ ] **Step 9: Commit.**

```bash
git add crates/kaiju-core/src/agent.rs crates/kaiju-daemon/src/settings.rs crates/kaiju-daemon/src/store.rs crates/kaiju-daemon/src/api.rs crates/kaiju-daemon/src/server.rs
git commit -m "feat(daemon): auto_continue opt-in + resume_at scheduling fields"
```

---

## Task 5: Arm and fire in the monitor

**Files:**
- Modify: `crates/kaiju-daemon/src/monitor.rs` (add `should_fire` + constant; integrate into `poll_once`)

**Interfaces:**
- Consumes: `AgentStatus::Limited`, `ParsedOutput.limit_reset`, `kaiju_adapters::limit::reset_to_utc`, `Store::set_resume_at`, `Store::mark_auto_continue_fired`, `Agent.{auto_continue,resume_at,last_fired}`, `TmuxManager::send_keys`.
- Produces: `pub fn should_fire(resume_at, last_fired, now, backoff_secs) -> bool`.

- [ ] **Step 1: Write the failing test.** In `crates/kaiju-daemon/src/monitor.rs` test module, add:

```rust
    #[test]
    fn should_fire_respects_timer_and_backoff() {
        let t = Utc::now();
        // Not armed.
        assert!(!should_fire(None, None, t, 60));
        // Armed, not due yet.
        assert!(!should_fire(Some(t + chrono::Duration::seconds(10)), None, t, 60));
        // Armed and due, never fired.
        assert!(should_fire(Some(t - chrono::Duration::seconds(1)), None, t, 60));
        // Due but within backoff of last fire.
        assert!(!should_fire(
            Some(t - chrono::Duration::seconds(1)),
            Some(t - chrono::Duration::seconds(30)),
            t,
            60
        ));
        // Due and past backoff.
        assert!(should_fire(
            Some(t - chrono::Duration::seconds(1)),
            Some(t - chrono::Duration::seconds(90)),
            t,
            60
        ));
    }
```

- [ ] **Step 2: Run test to verify it fails.**

Run: `cargo test -p kaiju-daemon should_fire_respects`
Expected: FAIL — `should_fire` undefined.

- [ ] **Step 3: Implement the pure fn + constant.** In `monitor.rs`, near the other `const` declarations (top of file) add:

```rust
/// Seconds after an auto-continue fire before we may fire again for the same
/// agent — stops a still-limited agent whose reset resolves to "now" from
/// tight-looping.
const MIN_RETRY_BACKOFF_SECS: i64 = 60;
```

Add the pure function (place it beside `resolve_status`):

```rust
/// Pure: should we inject `continue` for a limited agent right now?
pub fn should_fire(
    resume_at: Option<DateTime<Utc>>,
    last_fired: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    backoff_secs: i64,
) -> bool {
    let Some(resume_at) = resume_at else {
        return false;
    };
    if now < resume_at {
        return false;
    }
    if let Some(fired) = last_fired {
        if (now - fired).num_seconds() < backoff_secs {
            return false;
        }
    }
    true
}
```

- [ ] **Step 4: Run test to verify it passes.**

Run: `cargo test -p kaiju-daemon should_fire_respects`
Expected: PASS.

- [ ] **Step 5: Integrate into `poll_once`.** In `monitor.rs`, inside the per-agent loop, replace the existing status-write block:

```rust
        if next != agent.status {
            if notify::should_alert(agent.status, next) {
                notify::alert(agent, next);
            }
            state.store.update_status(&agent.id, next);
        }
```

with:

```rust
        if next != agent.status {
            if notify::should_alert(agent.status, next) {
                notify::alert(agent, next);
            }
            state.store.update_status(&agent.id, next);
        }

        // Auto-continue: arm a timer while limited, then inject `continue` at
        // reset. Self-correcting — if the agent is still limited afterward, the
        // next poll re-arms from the (possibly new) banner.
        if next == AgentStatus::Limited && agent.auto_continue {
            // Arm once; don't overwrite an already-armed timer.
            if agent.resume_at.is_none() {
                if let Some((time, tz)) = &parsed.limit_reset {
                    if let Some(at) = kaiju_adapters::limit::reset_to_utc(now, time, tz) {
                        state.store.set_resume_at(&agent.id, Some(at));
                    }
                }
            }
            // Re-read resume_at so an arm+fire can happen in the same poll when
            // the reset time is already in the past.
            let resume_at = state.store.get(&agent.id).and_then(|a| a.resume_at);
            if should_fire(resume_at, agent.last_fired, now, MIN_RETRY_BACKOFF_SECS) {
                let _ = TmuxManager::send_keys(&agent.session_name, "continue");
                state.store.mark_auto_continue_fired(&agent.id, now);
            }
        } else if agent.resume_at.is_some() {
            // Recovered / moved on — disarm.
            state.store.set_resume_at(&agent.id, None);
        }
```

Ensure the file imports `kaiju_adapters` (the daemon already depends on it; if there is no `use` needed because it's referenced by full path `kaiju_adapters::limit::...`, no import line is required — confirm `cargo build` is clean).

- [ ] **Step 6: Build + full daemon tests.**

Run: `cargo build -p kaiju-daemon && cargo test -p kaiju-daemon`
Expected: builds clean; all tests PASS.

- [ ] **Step 7: Commit.**

```bash
git add crates/kaiju-daemon/src/monitor.rs
git commit -m "feat(daemon): arm + fire auto-continue in the monitor loop"
```

---

## Task 6: Dashboard — status, controls, and badge

**Files:**
- Modify: `crates/kaiju-daemon/assets/dashboard-utils.js` (status maps)
- Modify: `crates/kaiju-daemon/assets/dashboard-utils.test.js` (test the maps)
- Modify: `crates/kaiju-daemon/src/dashboard.rs` (CSS + New-Agent + Preferences checkboxes)
- Modify: `crates/kaiju-daemon/assets/dashboard.js` (send/read the flag; resumes badge)

**Interfaces:**
- Consumes: API status string `"limited"`, `AgentResponse.resume_at`, `Settings.auto_continue`.

- [ ] **Step 1: Write the failing JS test.** In `dashboard-utils.test.js`, add:

```javascript
test("limited is an attention status and sorts near the top", () => {
  assert.ok(ATTENTION.has("limited"));
  assert.equal(statusLabel("limited"), "Limited");
  assert.ok(ORDER.limited < ORDER.running);
});
```

- [ ] **Step 2: Run test to verify it fails.**

Run: `node --test crates/kaiju-daemon/assets/`
Expected: FAIL — `ATTENTION.has("limited")` false / label mismatch.

- [ ] **Step 3: Update the status maps.** In `dashboard-utils.js`, replace the three lines:

```javascript
const ORDER = { waitingforinput: 0, limited: 1, stuck: 2, error: 3, starting: 4, running: 5, completed: 6, stopped: 7 };

const ATTENTION = new Set(["waitingforinput", "stuck", "limited"]);
```

and add `limited` to `STATUS_LABELS`:

```javascript
const STATUS_LABELS = {
  waitingforinput: "Waiting for input", running: "Running", starting: "Starting",
  stuck: "Stuck", error: "Error", completed: "Completed", stopped: "Stopped",
  limited: "Limited",
};
```

- [ ] **Step 4: Run test to verify it passes.**

Run: `node --test crates/kaiju-daemon/assets/`
Expected: PASS (all suites).

- [ ] **Step 5: Add `.s-limited` CSS.** In `crates/kaiju-daemon/src/dashboard.rs`, mirror the amber `waitingforinput` treatment. Add next to the existing status rules:
  - In the left-bar block (near `.agent-card.s-waitingforinput::before`), add `.agent-card.s-limited::before { background: var(--amber); }` — place it *before* the `.agent-card.selected::before` rule so selection still wins.
  - Add a card border/attention hint: `.agent-card.s-limited` should be covered by `.attention` styling (it will be, since a limited agent is in `ATTENTION` and the card gets the `attention` class from `cardHtml`). No extra card rule needed.
  - Add the dot + status chip colors alongside the other `.dot.s-*` / `.s-*` rules:

```css
  .dot.s-limited       { background: var(--amber);    animation: dotPulse 2s ease-in-out infinite; }
```

```css
  .s-limited           { background: var(--amber-weak); color: var(--amber); }
```

- [ ] **Step 6: Add the New-Agent + Preferences checkboxes.** In `dashboard.rs`, after the New-Agent auto-approve checkbox (`id="n-auto"`, ~line 1025) add:

```html
        <label class="check"><input type="checkbox" id="n-continue"> Auto-continue past usage limits (resume at reset)</label>
```

After the Preferences isolate checkbox (`id="pref-isolate"`, ~line 974) add:

```html
    <label class="check"><input type="checkbox" id="pref-continue"> Auto-continue new agents past usage limits</label>
```

- [ ] **Step 7: Wire the flag through `dashboard.js`.**
  - In `loadPrefs`, after the `pref-isolate` line add: `document.getElementById("pref-continue").checked = !!s.auto_continue;`
  - In `savePrefs`, in the `body` object add: `auto_continue: document.getElementById("pref-continue").checked,`
  - In `createAgent`, in the `body` object (next to `isolate`) add: `auto_continue: document.getElementById("n-continue").checked,`

- [ ] **Step 8: Show the "resumes" badge.** In `dashboard.js` `refreshDetail`, after the activity line is set, add (uses the existing `d-activity` element, appending when armed):

```javascript
  const upd = lastUpdated[selected];
  const resumeAt = (lastAgents.find(a => a.id === selected) || {}).resume_at;
  const activityEl = document.getElementById("d-activity");
  if (resumeAt) {
    const t = new Date(resumeAt);
    activityEl.textContent = "limited · resumes " +
      t.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  } else {
    activityEl.textContent = upd ? "active " + timeAgo(upd) : "";
  }
```

(Replace the existing single `document.getElementById("d-activity").textContent = ...` line with this block.)

- [ ] **Step 9: Verify assets compile into the daemon + JS tests pass.**

Run: `cargo build -p kaiju-daemon && node --test crates/kaiju-daemon/assets/`
Expected: builds clean; all JS tests PASS.

- [ ] **Step 10: Commit.**

```bash
git add crates/kaiju-daemon/assets/dashboard-utils.js crates/kaiju-daemon/assets/dashboard-utils.test.js crates/kaiju-daemon/src/dashboard.rs crates/kaiju-daemon/assets/dashboard.js
git commit -m "feat(dashboard): limited status, auto-continue toggle, resumes badge"
```

---

## Task 7: Ship gate

**Files:** none (verification only).

- [ ] **Step 1: Format.**

Run: `cargo fmt --all && cargo fmt --all --check`
Expected: no diff.

- [ ] **Step 2: Clippy.**

Run: `cargo clippy --workspace --all-targets`
Expected: no warnings.

- [ ] **Step 3: Rust tests.**

Run: `cargo test --workspace`
Expected: all PASS.

- [ ] **Step 4: JS tests.**

Run: `node --test crates/kaiju-daemon/assets/`
Expected: all PASS.

- [ ] **Step 5: Manual verification (optional but recommended).** Use the `verify` skill or drive it by hand: create an agent with auto-continue on, simulate the banner by writing the sample line into its tmux pane, and confirm the card shows `Limited`, the detail shows `resumes 5:30pm`, and (with a near-future reset) `continue` is injected. Do NOT restart the user's live daemon — verify on a separate `KAIJU_HTTP_ADDR`/port instance.

- [ ] **Step 6: Final commit if fmt changed anything, otherwise done.**

```bash
git status
# if fmt produced changes:
git add -A && git commit -m "chore: fmt"
```

---

## Self-Review notes

- **Spec coverage:** A=Task 2; B=Tasks 1,3; C=Task 4 + Task 6 (UI); D=Tasks 4 (fields/store) + 5 (arm/fire); E=Task 6. Testing section → Tasks 1,3,4,5,6 tests + Task 7 gate. Dependencies (chrono-tz) → Task 1.
- **Deviation from spec (improvement):** `resume_at`/`last_fired` are `#[serde(default)]` fields on `Agent`, so they persist with the store. This is strictly better than the spec's "runtime-only" note — a daemon restart mid-wait keeps the armed timer instead of relying on re-detection. No downside; the open-question limitation in the spec is resolved.
- **Type consistency:** `set_resume_at` / `mark_auto_continue_fired` / `should_fire` / `reset_to_utc` / `parse_limit_reset` / `limit_reset` used with identical signatures across producing and consuming tasks.
