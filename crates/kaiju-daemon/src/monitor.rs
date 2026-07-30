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

/// A fresh "waiting for input" signal from the parser is only trusted once the
/// pane has been quiet this long. A prompt-looking line that flickers past while
/// the agent is still streaming output should not raise a "needs input" alert.
const WAIT_CONFIRM_SECS: i64 = 3;

/// Seconds after an auto-continue fire before we may fire again for the same
/// agent — stops a still-limited agent whose reset resolves to "now" from
/// tight-looping.
const MIN_RETRY_BACKOFF_SECS: i64 = 60;

/// A limit banner is only acted on once the pane has been quiet this long — so
/// we don't arm off a banner glimpsed while the agent is still streaming output
/// (e.g. right after it resumes). Mirrors `WAIT_CONFIRM_SECS`.
const LIMIT_CONFIRM_SECS: i64 = 3;

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

/// Pure: decide an agent's next status from the parser signal and idle time.
///
/// - A Running agent (whether the parser said so, or it was already Running and
///   the parser is silent) that has been idle past `threshold` becomes `Stuck`.
/// - A `Stuck` agent whose output has moved again (idle below `threshold`)
///   recovers to `Running`.
/// - A *new* waiting signal is held back until output has been quiet for
///   `wait_confirm` seconds, so a prompt-looking line glimpsed mid-stream does
///   not raise a false "needs input" alert. An agent already waiting stays so.
/// - Any other explicit non-Running parser signal (error, completed) passes
///   through unchanged, so a waiting agent is never marked stuck.
pub fn resolve_status(
    current: AgentStatus,
    parsed: Option<AgentStatus>,
    idle_secs: i64,
    threshold: i64,
    wait_confirm: i64,
) -> AgentStatus {
    let base = parsed.unwrap_or(current);
    match base {
        // Only trust a freshly-detected prompt once the pane has actually gone
        // quiet; until then the agent is still considered active.
        AgentStatus::WaitingForInput
            if current != AgentStatus::WaitingForInput && idle_secs < wait_confirm =>
        {
            AgentStatus::Running
        }
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
        state.store.update_metrics(&agent.id, metrics.clone());

        // Enforce a configured token/cost budget: stop the agent once it's hit.
        let budget = state
            .settings
            .read()
            .expect("settings lock")
            .budget_exceeded(&metrics);
        if let Some(reason) = budget {
            tracing::info!("stopping agent {} — {reason}", agent.id);
            let _ = crate::server::stop_agent_internal(state, &agent.id);
            continue;
        }

        // Track output changes to measure idle time.
        let fp = fingerprint(&output);
        // Mirror the latest capture to disk (on first sight and whenever it
        // changes) so logs survive the session ending.
        if activity.get(&agent.id).map(|r| r.fingerprint) != Some(fp) {
            crate::logstore::save(&agent.id, &output);
        }
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

        let mut next = resolve_status(
            agent.status,
            parsed.status,
            idle_secs,
            STUCK_THRESHOLD_SECS,
            WAIT_CONFIRM_SECS,
        );

        // A usage/session limit takes priority: the session is alive but stalled
        // until reset. Detect it on the *visible* pane (no scrollback) so a
        // banner that has scrolled into history after the agent resumed no
        // longer matches — a staleness signal independent of terminal height.
        let limit_reset = TmuxManager::capture_pane_visible(&agent.session_name)
            .ok()
            .and_then(|visible| adapter.detect_limit(&visible));
        if limit_reset.is_some() {
            next = AgentStatus::Limited;
        }

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
            // Arm once the stream has actually stopped (don't act on a banner
            // seen mid-output), and never overwrite an already-armed timer.
            if idle_secs >= LIMIT_CONFIRM_SECS && agent.resume_at.is_none() {
                if let Some((time, tz)) = &limit_reset {
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
    }

    // Drop activity records for agents no longer being polled.
    let live: std::collections::HashSet<&str> =
        started_agents.iter().map(|a| a.id.as_str()).collect();
    activity.retain(|id, _| live.contains(id.as_str()));
}

/// How often the persisted-log retention purge runs.
const LOG_PURGE_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Run the monitor loop forever, polling every `interval`.
pub async fn run_monitor(state: AppState, interval: Duration) {
    debug!("monitor started, interval={:?}", interval);
    let mut activity: HashMap<String, OutputActivity> = HashMap::new();
    let mut last_purge: Option<std::time::Instant> = None;
    loop {
        tokio::time::sleep(interval).await;
        poll_once(&state, &mut activity);

        // Rotate persisted logs and analytics records past retention, daily.
        if last_purge.is_none_or(|at| at.elapsed() >= LOG_PURGE_INTERVAL) {
            let logs = crate::logstore::purge_expired();
            let records = crate::analytics::prune_expired();
            if logs > 0 || records > 0 {
                tracing::info!(
                    "log rotation: removed {logs} expired log file(s), {records} analytics record(s)"
                );
            }
            last_purge = Some(std::time::Instant::now());
        }
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
    const W: i64 = 3;

    #[test]
    fn running_and_idle_becomes_stuck() {
        // Parser silent, agent was Running, idle past threshold.
        let next = resolve_status(AgentStatus::Running, None, T, T, W);
        assert_eq!(next, AgentStatus::Stuck);
    }

    #[test]
    fn running_with_recent_output_stays_running() {
        let next = resolve_status(AgentStatus::Running, Some(AgentStatus::Running), 3, T, W);
        assert_eq!(next, AgentStatus::Running);
    }

    #[test]
    fn stuck_recovers_when_output_moves() {
        // Output changed (idle reset to ~0), parser silent.
        let next = resolve_status(AgentStatus::Stuck, None, 0, T, W);
        assert_eq!(next, AgentStatus::Running);
    }

    #[test]
    fn stuck_stays_stuck_while_idle() {
        let next = resolve_status(AgentStatus::Stuck, None, T + 30, T, W);
        assert_eq!(next, AgentStatus::Stuck);
    }

    #[test]
    fn waiting_is_never_marked_stuck() {
        let next = resolve_status(
            AgentStatus::WaitingForInput,
            Some(AgentStatus::WaitingForInput),
            10_000,
            T,
            W,
        );
        assert_eq!(next, AgentStatus::WaitingForInput);
    }

    #[test]
    fn explicit_completion_passes_through() {
        let next = resolve_status(AgentStatus::Running, Some(AgentStatus::Completed), T, T, W);
        assert_eq!(next, AgentStatus::Completed);
    }

    #[test]
    fn fresh_waiting_signal_is_held_until_output_is_quiet() {
        // A prompt-looking line appeared but output is still moving (idle below
        // the confirm window): treat the agent as still active, not waiting.
        let next = resolve_status(
            AgentStatus::Running,
            Some(AgentStatus::WaitingForInput),
            W - 1,
            T,
            W,
        );
        assert_eq!(next, AgentStatus::Running);
    }

    #[test]
    fn waiting_signal_is_trusted_once_output_is_quiet() {
        // Same prompt, but the pane has now been quiet past the confirm window.
        let next = resolve_status(
            AgentStatus::Running,
            Some(AgentStatus::WaitingForInput),
            W,
            T,
            W,
        );
        assert_eq!(next, AgentStatus::WaitingForInput);
    }

    #[test]
    fn already_waiting_stays_waiting_even_if_output_just_moved() {
        // Confirmed waiting agents are not bounced back to Running by a brief
        // change (e.g. the prompt redrawing), regardless of idle time.
        let next = resolve_status(
            AgentStatus::WaitingForInput,
            Some(AgentStatus::WaitingForInput),
            0,
            T,
            W,
        );
        assert_eq!(next, AgentStatus::WaitingForInput);
    }

    #[test]
    fn should_fire_respects_timer_and_backoff() {
        let t = Utc::now();
        // Not armed.
        assert!(!should_fire(None, None, t, 60));
        // Armed, not due yet.
        assert!(!should_fire(
            Some(t + chrono::Duration::seconds(10)),
            None,
            t,
            60
        ));
        // Armed and due, never fired.
        assert!(should_fire(
            Some(t - chrono::Duration::seconds(1)),
            None,
            t,
            60
        ));
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
}
