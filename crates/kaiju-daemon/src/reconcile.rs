//! Startup reconciliation between persisted agent state and live tmux sessions.
//!
//! When the daemon restarts it reloads agents from disk, but their tmux
//! sessions may have ended while it was down. This pure function identifies
//! agents that are recorded as active yet have no live session, so the caller
//! can mark them stopped.

use kaiju_core::agent::Agent;

/// Return the ids of agents that are recorded active but whose tmux session is
/// no longer present in `live_sessions`.
pub fn orphaned_active_ids(agents: &[Agent], live_sessions: &[String]) -> Vec<String> {
    agents
        .iter()
        .filter(|agent| agent.status.is_active())
        .filter(|agent| !live_sessions.iter().any(|name| name == &agent.session_name))
        .map(|agent| agent.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kaiju_core::agent::{Agent, AgentConfig, AgentStatus, AgentType};
    use std::path::PathBuf;

    fn agent_with_status(status: AgentStatus) -> Agent {
        let mut agent = Agent::new(AgentConfig {
            agent_type: AgentType::Claude,
            model: None,
            workspace: PathBuf::from("/tmp"),
            prompt: None,
            extra_args: vec![],
        });
        agent.update_status(status);
        agent
    }

    #[test]
    fn active_agent_without_session_is_orphaned() {
        let agent = agent_with_status(AgentStatus::Running);
        let orphans = orphaned_active_ids(std::slice::from_ref(&agent), &[]);
        assert_eq!(orphans, vec![agent.id]);
    }

    #[test]
    fn active_agent_with_live_session_is_kept() {
        let agent = agent_with_status(AgentStatus::Running);
        let live = vec![agent.session_name.clone()];
        let orphans = orphaned_active_ids(std::slice::from_ref(&agent), &live);
        assert!(orphans.is_empty());
    }

    #[test]
    fn terminal_agent_is_never_orphaned() {
        let agent = agent_with_status(AgentStatus::Completed);
        let orphans = orphaned_active_ids(std::slice::from_ref(&agent), &[]);
        assert!(orphans.is_empty());
    }
}
