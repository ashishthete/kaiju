//! Background monitor that keeps agent status and metrics up to date.
//!
//! Responsibilities are deliberately split:
//! - [`updated_metrics`] is a pure function: given the previous metrics, the
//!   adapter's parsed output, and the current time, it computes the new
//!   metrics. It performs no IO and is fully unit-tested.
//! - [`poll_once`] / [`run_monitor`] do the IO: capture tmux output, run the
//!   adapter parser, and write results back to the store.

use chrono::{DateTime, Utc};
use kaiju_core::adapter::ParsedOutput;
use kaiju_core::agent::{AgentMetrics, AgentStatus};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::time::Duration;
use tracing::debug;

use crate::notify;
use crate::server::AppState;
use crate::tmux::TmuxManager;

/// Number of trailing pane lines to capture when polling an agent.
const CAPTURE_LINES: u32 = 200;

/// A Running agent whose output has not changed for this many seconds is
/// considered stuck.
const STUCK_THRESHOLD_SECS: i64 = 120;

/// Per-agent record of the last distinct output seen, used to measure idle time.
pub(crate) struct OutputActivity {
    fingerprint: u64,
    since: DateTime<Utc>,
}

/// A cheap content fingerprint of pane output, to detect whether it changed.
fn fingerprint(output: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    output.hash(&mut hasher);
    hasher.finish()
}

/// Pure: decide an agent's next status from the parser signal and idle time.
///
/// - A Running agent (whether the parser said so, or it was already Running and
///   the parser is silent) that has been idle past `threshold` becomes `Stuck`.
/// - A `Stuck` agent whose output has moved again (idle below `threshold`)
///   recovers to `Running`.
/// - Any explicit non-Running parser signal (waiting, error, completed) passes
///   through unchanged, so a waiting agent is never marked stuck.
pub fn resolve_status(
    current: AgentStatus,
    parsed: Option<AgentStatus>,
    idle_secs: i64,
    threshold: i64,
) -> AgentStatus {
    let base = parsed.unwrap_or(current);
    match base {
        AgentStatus::Running if idle_secs >= threshold => AgentStatus::Stuck,
        AgentStatus::Stuck if idle_secs < threshold => AgentStatus::Running,
        other => other,
    }
}

/// Pure: compute updated metrics from elapsed time and parsed output.
///
/// Runtime is derived from `started_at`; if the agent has not started, the
/// previous runtime is kept. Tokens and cost are taken from the parsed output
/// when present, otherwise the previous value is retained (parsing is
/// best-effort and a given capture may not contain those lines).
pub fn updated_metrics(
    started_at: Option<DateTime<Utc>>,
    previous: &AgentMetrics,
    parsed: &ParsedOutput,
    now: DateTime<Utc>,
) -> AgentMetrics {
    let runtime_secs = match started_at {
        Some(started) => (now - started).num_seconds().max(0) as u64,
        None => previous.runtime_secs,
    };

    AgentMetrics {
        runtime_secs,
        tokens_used: parsed.tokens_used.or(previous.tokens_used),
        estimated_cost_usd: parsed.estimated_cost_usd.or(previous.estimated_cost_usd),
    }
}

/// Poll every started, non-terminal agent once and update its state.
///
/// `activity` is the monitor's ephemeral memory of when each agent's output last
/// changed; it is used to detect stuck agents and is pruned to live agents.
pub(crate) fn poll_once(state: &AppState, activity: &mut HashMap<String, OutputActivity>) {
    let started_agents: Vec<_> = state
        .store
        .list_active()
        .into_iter()
        .filter(|a| a.started_at.is_some())
        .collect();

    for agent in &started_agents {
        let Some(adapter) = state.adapters.get(&agent.agent_type) else {
            continue;
        };

        let output = match TmuxManager::capture_pane(&agent.session_name, CAPTURE_LINES) {
            Ok(output) => output,
            Err(_) => {
                // Capture failed. If the session is gone, the agent process —
                // which is the session's main process — exited on its own. A
                // manual stop already set Stopped and left the active set, so an
                // *active* agent whose session vanished completed naturally.
                if !TmuxManager::session_exists(&agent.session_name) {
                    state.store.update_status(&agent.id, AgentStatus::Completed);
                }
                continue;
            }
        };

        let now = Utc::now();
        let mut parsed = adapter.parse_output(&output);

        // Prefer authoritative token/cost metrics from the CLI's own transcript
        // (e.g. Claude's session JSONL) over the screen-scraping heuristics.
        if let Some(started) = agent.started_at {
            let run_dir = agent
                .worktree_path
                .clone()
                .unwrap_or_else(|| agent.workspace.clone());
            if let Some(precise) = adapter.read_metrics(&run_dir, started.timestamp()) {
                parsed.tokens_used = precise.tokens_used.or(parsed.tokens_used);
                parsed.estimated_cost_usd =
                    precise.estimated_cost_usd.or(parsed.estimated_cost_usd);
            }
        }

        let metrics = updated_metrics(agent.started_at, &agent.metrics, &parsed, now);
        state.store.update_metrics(&agent.id, metrics);

        // Track output changes to measure idle time.
        let fp = fingerprint(&output);
        let record = activity.entry(agent.id.clone()).or_insert(OutputActivity {
            fingerprint: fp,
            since: now,
        });
        if record.fingerprint != fp {
            *record = OutputActivity {
                fingerprint: fp,
                since: now,
            };
        }
        let idle_secs = (now - record.since).num_seconds();

        let next = resolve_status(agent.status, parsed.status, idle_secs, STUCK_THRESHOLD_SECS);
        if next != agent.status {
            if notify::should_alert(agent.status, next) {
                notify::alert(agent, next);
            }
            state.store.update_status(&agent.id, next);
        }
    }

    // Drop activity records for agents no longer being polled.
    let live: std::collections::HashSet<&str> =
        started_agents.iter().map(|a| a.id.as_str()).collect();
    activity.retain(|id, _| live.contains(id.as_str()));
}

/// Run the monitor loop forever, polling every `interval`.
pub async fn run_monitor(state: AppState, interval: Duration) {
    debug!("monitor started, interval={:?}", interval);
    let mut activity: HashMap<String, OutputActivity> = HashMap::new();
    loop {
        tokio::time::sleep(interval).await;
        poll_once(&state, &mut activity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_metrics() -> AgentMetrics {
        AgentMetrics {
            runtime_secs: 5,
            tokens_used: Some(100),
            estimated_cost_usd: Some(0.10),
        }
    }

    #[test]
    fn runtime_is_derived_from_started_at() {
        let started = Utc::now();
        let now = started + chrono::Duration::seconds(30);
        let parsed = ParsedOutput::default();

        let metrics = updated_metrics(Some(started), &base_metrics(), &parsed, now);

        assert_eq!(metrics.runtime_secs, 30);
    }

    #[test]
    fn runtime_kept_when_not_started() {
        let parsed = ParsedOutput::default();
        let metrics = updated_metrics(None, &base_metrics(), &parsed, Utc::now());
        assert_eq!(metrics.runtime_secs, 5);
    }

    #[test]
    fn negative_elapsed_clamps_to_zero() {
        let started = Utc::now();
        let now = started - chrono::Duration::seconds(10);
        let parsed = ParsedOutput::default();

        let metrics = updated_metrics(Some(started), &base_metrics(), &parsed, now);

        assert_eq!(metrics.runtime_secs, 0);
    }

    #[test]
    fn parsed_tokens_and_cost_override_previous() {
        let started = Utc::now();
        let parsed = ParsedOutput {
            status: None,
            tokens_used: Some(500),
            estimated_cost_usd: Some(1.25),
        };

        let metrics = updated_metrics(Some(started), &base_metrics(), &parsed, started);

        assert_eq!(metrics.tokens_used, Some(500));
        assert_eq!(metrics.estimated_cost_usd, Some(1.25));
    }

    #[test]
    fn previous_tokens_and_cost_retained_when_not_parsed() {
        let started = Utc::now();
        let parsed = ParsedOutput::default();

        let metrics = updated_metrics(Some(started), &base_metrics(), &parsed, started);

        assert_eq!(metrics.tokens_used, Some(100));
        assert_eq!(metrics.estimated_cost_usd, Some(0.10));
    }

    const T: i64 = 120;

    #[test]
    fn running_and_idle_becomes_stuck() {
        // Parser silent, agent was Running, idle past threshold.
        let next = resolve_status(AgentStatus::Running, None, T, T);
        assert_eq!(next, AgentStatus::Stuck);
    }

    #[test]
    fn running_with_recent_output_stays_running() {
        let next = resolve_status(AgentStatus::Running, Some(AgentStatus::Running), 3, T);
        assert_eq!(next, AgentStatus::Running);
    }

    #[test]
    fn stuck_recovers_when_output_moves() {
        // Output changed (idle reset to ~0), parser silent.
        let next = resolve_status(AgentStatus::Stuck, None, 0, T);
        assert_eq!(next, AgentStatus::Running);
    }

    #[test]
    fn stuck_stays_stuck_while_idle() {
        let next = resolve_status(AgentStatus::Stuck, None, T + 30, T);
        assert_eq!(next, AgentStatus::Stuck);
    }

    #[test]
    fn waiting_is_never_marked_stuck() {
        let next = resolve_status(
            AgentStatus::WaitingForInput,
            Some(AgentStatus::WaitingForInput),
            10_000,
            T,
        );
        assert_eq!(next, AgentStatus::WaitingForInput);
    }

    #[test]
    fn explicit_completion_passes_through() {
        let next = resolve_status(AgentStatus::Running, Some(AgentStatus::Completed), T, T);
        assert_eq!(next, AgentStatus::Completed);
    }
}
