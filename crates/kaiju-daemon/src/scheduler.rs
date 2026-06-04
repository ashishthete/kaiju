//! The task scheduler — a bounded agent pool that drains the queue.
//!
//! Pure decisions ([`slots_available`], [`task_outcome`]) are unit-tested; the
//! loop ([`schedule_once`] / [`run_scheduler`]) does the IO: finalize tasks
//! whose agent finished, then start queued tasks into any free slots.

use kaiju_core::agent::AgentStatus;
use kaiju_core::task::TaskStatus;
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::server::{self, AppState};

/// Default number of agents the pool runs concurrently.
pub const DEFAULT_CONCURRENCY: usize = 2;

/// Pure: how many new tasks may start, given the running count and the limit.
pub fn slots_available(running: usize, concurrency: usize) -> usize {
    concurrency.saturating_sub(running)
}

/// Pure: the task outcome implied by an agent status, or `None` if the agent is
/// not finished yet.
pub fn task_outcome(agent_status: AgentStatus) -> Option<TaskStatus> {
    match agent_status {
        AgentStatus::Completed => Some(TaskStatus::Done),
        AgentStatus::Error | AgentStatus::Stopped => Some(TaskStatus::Failed),
        _ => None,
    }
}

/// One scheduling pass: reconcile running tasks, then fill free slots.
pub fn schedule_once(state: &AppState, concurrency: usize) {
    // 1. Finalize running tasks whose agent reached a terminal state.
    for task in state.tasks.running() {
        let Some(agent_id) = &task.agent_id else {
            continue;
        };
        match state.store.get(agent_id) {
            Some(agent) => {
                if let Some(outcome) = task_outcome(agent.status) {
                    state.tasks.finish(&task.id, outcome);
                    info!("task {} -> {:?}", task.id, outcome);
                }
            }
            None => {
                state
                    .tasks
                    .fail(&task.id, "agent no longer exists".to_string());
            }
        }
    }

    // 2. Start queued tasks into any free slots.
    let slots = slots_available(state.tasks.count_running(), concurrency);
    for task in state.tasks.next_queued(slots) {
        match server::spawn_started_agent(state, &task.spec.to_config(), task.spec.isolate) {
            Ok(agent_id) => {
                state.tasks.mark_running(&task.id, agent_id.clone());
                info!("task {} started as agent {}", task.id, agent_id);
            }
            Err(e) => {
                warn!("task {} failed to start: {e}", task.id);
                state.tasks.fail(&task.id, e.to_string());
            }
        }
    }
}

/// Run the scheduler loop forever.
pub async fn run_scheduler(state: AppState, interval: Duration, concurrency: usize) {
    debug!("scheduler started, concurrency={concurrency}");
    loop {
        tokio::time::sleep(interval).await;
        schedule_once(&state, concurrency);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_available_subtracts_running() {
        assert_eq!(slots_available(0, 2), 2);
        assert_eq!(slots_available(1, 2), 1);
        assert_eq!(slots_available(2, 2), 0);
    }

    #[test]
    fn slots_available_never_underflows() {
        assert_eq!(slots_available(5, 2), 0);
    }

    #[test]
    fn completed_agent_means_task_done() {
        assert_eq!(task_outcome(AgentStatus::Completed), Some(TaskStatus::Done));
    }

    #[test]
    fn error_or_stopped_means_task_failed() {
        assert_eq!(task_outcome(AgentStatus::Error), Some(TaskStatus::Failed));
        assert_eq!(task_outcome(AgentStatus::Stopped), Some(TaskStatus::Failed));
    }

    #[test]
    fn running_agent_has_no_outcome_yet() {
        assert_eq!(task_outcome(AgentStatus::Running), None);
        assert_eq!(task_outcome(AgentStatus::WaitingForInput), None);
        assert_eq!(task_outcome(AgentStatus::Stuck), None);
    }
}
