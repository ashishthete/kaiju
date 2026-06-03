//! Batch executor: run an agent CLI in structured (non-interactive) mode and
//! drive its status/metrics from the JSON event stream rather than scraping a
//! terminal. Used for fire-and-forget agents that need precise cost/tokens.
//!
//! The decision logic lives in the adapters (`structured_command`, `parse_event`)
//! and in `monitor::updated_metrics`/`notify`; this module is the thin IO loop
//! that spawns the process and applies parsed events to the store.

use std::process::Stdio;

use chrono::Utc;
use nexus_core::adapter::ParsedOutput;
use nexus_core::agent::{AgentStatus, AgentType};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::runtime::Handle;
use tracing::{info, warn};

use crate::monitor::updated_metrics;
use crate::notify;
use crate::server::AppState;

/// Spawn the batch runner for an agent onto the current Tokio runtime. The agent
/// must already be marked started so runtime is measured from launch.
pub fn spawn_batch(state: AppState, agent_id: String, command: String, agent_type: AgentType) {
    let Ok(handle) = Handle::try_current() else {
        warn!("no runtime available to run batch agent {agent_id}");
        return;
    };
    handle.spawn(run_batch(state, agent_id, command, agent_type));
}

async fn run_batch(state: AppState, agent_id: String, command: String, agent_type: AgentType) {
    let mut child = match Command::new("sh")
        .arg("-c")
        .arg(&command)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            warn!("batch agent {agent_id} failed to start: {e}");
            state.store.update_status(&agent_id, AgentStatus::Error);
            return;
        }
    };

    let Some(stdout) = child.stdout.take() else {
        state.store.update_status(&agent_id, AgentStatus::Error);
        return;
    };
    let mut lines = BufReader::new(stdout).lines();

    while let Ok(Some(line)) = lines.next_line().await {
        // Cooperative cancellation: a stop request kills the process.
        if matches!(
            state.store.get(&agent_id).map(|a| a.status),
            Some(AgentStatus::Stopped)
        ) {
            let _ = child.start_kill();
            break;
        }

        let parsed = state
            .adapters
            .get(&agent_type)
            .and_then(|adapter| adapter.parse_event(&line));
        if let Some(parsed) = parsed {
            apply_event(&state, &agent_id, parsed);
        }
    }

    // Finalize: if no `result` event already set a terminal status, derive one
    // from the process exit.
    let exit_ok = matches!(child.wait().await, Ok(status) if status.success());
    if let Some(agent) = state.store.get(&agent_id) {
        if !agent.status.is_terminal() {
            let status = if exit_ok {
                AgentStatus::Completed
            } else {
                AgentStatus::Error
            };
            state.store.update_status(&agent_id, status);
        }
    }
    info!("batch agent {agent_id} finished");
}

/// Apply one parsed event to the store: update metrics, and status with an alert
/// on transitions that need a human. Mirrors the monitor's handling so behavior
/// is identical whether an agent is interactive or batch.
fn apply_event(state: &AppState, agent_id: &str, parsed: ParsedOutput) {
    let Some(agent) = state.store.get(agent_id) else {
        return;
    };
    let metrics = updated_metrics(agent.started_at, &agent.metrics, &parsed, Utc::now());
    state.store.update_metrics(agent_id, metrics);

    if let Some(status) = parsed.status {
        if notify::should_alert(agent.status, status) {
            notify::alert(&agent, status);
        }
        state.store.update_status(agent_id, status);
    }
}
