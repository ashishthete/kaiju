//! Background monitor that keeps agent status and metrics up to date.
//!
//! Responsibilities are deliberately split:
//! - [`updated_metrics`] is a pure function: given the previous metrics, the
//!   adapter's parsed output, and the current time, it computes the new
//!   metrics. It performs no IO and is fully unit-tested.
//! - [`poll_once`] / [`run_monitor`] do the IO: capture tmux output, run the
//!   adapter parser, and write results back to the store.

use chrono::{DateTime, Utc};
use nexus_core::adapter::ParsedOutput;
use nexus_core::agent::{AgentMetrics, AgentStatus};
use std::time::Duration;
use tracing::debug;

use crate::notify;
use crate::server::AppState;
use crate::tmux::TmuxManager;

/// Number of trailing pane lines to capture when polling an agent.
const CAPTURE_LINES: u32 = 200;

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
pub fn poll_once(state: &AppState) {
    let started_agents = state
        .store
        .list_active()
        .into_iter()
        .filter(|a| a.started_at.is_some());

    for agent in started_agents {
        let Some(adapter) = state.adapters.get(&agent.agent_type) else {
            continue;
        };

        let output = match TmuxManager::capture_pane(&agent.session_name, CAPTURE_LINES) {
            Ok(output) => output,
            Err(_) => {
                // Capture failed. If the session is gone, the process exited;
                // mark the agent stopped so it leaves the active set.
                if !TmuxManager::session_exists(&agent.session_name) {
                    state.store.update_status(&agent.id, AgentStatus::Stopped);
                }
                continue;
            }
        };

        let parsed = adapter.parse_output(&output);
        let metrics = updated_metrics(agent.started_at, &agent.metrics, &parsed, Utc::now());
        state.store.update_metrics(&agent.id, metrics);

        if let Some(status) = parsed.status {
            if notify::should_alert(agent.status, status) {
                notify::alert(&agent, status);
            }
            state.store.update_status(&agent.id, status);
        }
    }
}

/// Run the monitor loop forever, polling every `interval`.
pub async fn run_monitor(state: AppState, interval: Duration) {
    debug!("monitor started, interval={:?}", interval);
    loop {
        tokio::time::sleep(interval).await;
        poll_once(&state);
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
}
